use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::sources::Changing;

//@show periods: declaring refresh settings
use amethystate::{Field, Store, amethystate};

#[amethystate(prefix = "refresh")]
pub struct RefreshSettings {
    #[amestate(default = 1000u64)]
    pub interval_ms: u64,
}
//@show-end

//@show periods: an amethystate field adapter

pub struct Milliseconds(Field<u64>);

impl Changing for Milliseconds {
    type Value = u64;

    fn now(&self) -> Option<u64> {
        Some(self.0.get())
    }

    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        Box::new(self.0.subscribe(move |_| changed()))
    }
}
//@show-end

//@show periods: installing an amethystate timer
feature! {
    pub ConfiguredPolling {
        exports { Count }
    }
}

#[installs]
fn configured_polling(cx: &Install, store: Store) -> anyhow::Result<ConfiguredPolling> {
    let settings = RefreshSettings::try_new_with(&store)?;
    let count = cx.state::<Count>().plain();
    let period = Period::follows(Milliseconds(settings.interval_ms()), Duration::from_millis);
    cx.repeat(period, move || count.push(()));
    Ok(ConfiguredPolling(count))
}
//@show-end

//@show periods: a store for the example
plugin! {
    pub MemoryStore {
        provides { Store }
    }
}

#[builds]
fn memory_store(_app: &mut PluginBuilder) -> anyhow::Result<MemoryStore> {
    let store = amethystate::StoreBuilder::in_memory().build()?;
    Ok(MemoryStore(store))
}
//@show-end

//@show periods: changing an amethystate field rearms the timer
#[guinea::test]
fn changing_an_amethystate_field_restarts_the_timer(h: &mut Harness) {
    let MemoryStore(store) = h.plugin::<MemoryStore>(()).unwrap();
    let settings = RefreshSettings::new_with(&store);
    let interval = settings.interval_ms();
    h.install::<ConfiguredPolling>(&()).unwrap();
    h.settled();
    h.advance(Duration::from_millis(500));

    interval.set(2000);
    h.settled();
    h.advance(Duration::from_millis(1500));
    assert_eq!(h.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 1);
    assert_eq!(interval.get(), 2000);
    h.advance(Duration::from_millis(1999));
    assert_eq!(h.state::<Count>().0, 1);
    h.advance(Duration::from_millis(1));
    assert_eq!(h.state::<Count>().0, 2);
}
//@show-end

#[test]
fn the_timer_example_has_valid_dependency_declarations() {
    guinea::app::validate_declarations();
}

#[guinea::test]
fn an_amethystate_write_from_another_thread_rearms_the_timer(h: &mut Harness) {
    let MemoryStore(store) = h.plugin::<MemoryStore>(()).unwrap();
    let settings = RefreshSettings::new_with(&store);
    let interval = settings.interval_ms();
    h.install::<ConfiguredPolling>(&()).unwrap();
    h.settled();
    let written = interval.clone();
    std::thread::spawn(move || written.set(250)).join().unwrap();
    h.settled();
    h.advance(Duration::from_millis(249));
    assert_eq!(h.state::<Count>().0, 0);
    h.advance(Duration::from_millis(1));
    assert_eq!(h.state::<Count>().0, 1);
}

#[guinea::test]
fn an_amethystate_subscription_guard_unsubscribes_on_drop(_h: &mut Harness) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let store = amethystate::StoreBuilder::in_memory().build().unwrap();
    let settings = RefreshSettings::new_with(&store);
    let interval = settings.interval_ms();
    let changes = Milliseconds(interval.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let guard = changes.watch(Box::new(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    }));
    interval.set(500);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(guard);
    interval.set(250);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

//@show periods: an observable interval
type Changed = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct IntervalState {
    value: Option<Duration>,
    next: u64,
    watchers: BTreeMap<u64, Changed>,
}

#[derive(Clone, Default)]
pub struct Interval(Arc<Mutex<IntervalState>>);

impl Interval {
    pub fn set(&self, value: Option<Duration>) {
        let watchers: Vec<_> = {
            let mut state = self.0.lock().unwrap();
            state.value = value;
            state.watchers.values().cloned().collect()
        };
        for changed in watchers {
            changed();
        }
    }
}

struct Unwatch(Interval, u64);

impl Drop for Unwatch {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().watchers.remove(&self.1);
    }
}

impl Changing for Interval {
    type Value = Duration;

    fn now(&self) -> Option<Duration> {
        self.0.lock().unwrap().value
    }

    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        let mut state = self.0.lock().unwrap();
        state.next += 1;
        let id = state.next;
        state.watchers.insert(id, Arc::from(changed));
        Box::new(Unwatch(self.clone(), id))
    }
}
//@show-end

//@show periods: counting refreshes
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

#[reducer]
fn count(this: &mut Count, _: ()) {
    this.0 += 1;
}
//@show-end

//@show periods: installing a dynamic timer
feature! {
    pub Polling {
        exports { Count }
    }
}

#[installs]
fn polling(cx: &Install<Interval>) -> anyhow::Result<Polling> {
    let count = cx.state::<Count>().plain();
    let period = Period::follows(cx.params().clone(), std::convert::identity);
    cx.repeat(period, move || count.push(()));
    Ok(Polling(count))
}
//@show-end

//@show periods: a change restarts the wait
#[guinea::test]
fn a_period_change_starts_a_new_wait_instead_of_finishing_the_old_one(h: &mut Harness) {
    let interval = Interval::default();
    h.install::<Polling>(&interval).unwrap();
    h.advance(Duration::from_secs(10));
    assert_eq!(h.state::<Count>().0, 0);

    interval.set(Some(Duration::from_secs(1)));
    h.settled();
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 0);
    interval.set(Some(Duration::from_secs(2)));
    h.settled();
    h.advance(Duration::from_millis(1500));
    assert_eq!(h.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 1);
}
//@show-end

//@show periods: changes while asleep
#[guinea::test]
fn a_changed_period_during_sleep_does_not_replay_missed_ticks(h: &mut Harness) {
    let interval = Interval::default();
    interval.set(Some(Duration::from_secs(1)));
    let page = h.child();
    page.install::<Polling>(&interval).unwrap();
    h.settled();
    page.scope().sleep();
    interval.set(Some(Duration::from_millis(500)));
    h.advance(Duration::from_secs(5));
    assert_eq!(page.state::<Count>().0, 0);

    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Count>().0, 1);
    h.advance(Duration::from_millis(500));
    assert_eq!(page.state::<Count>().0, 2);

    page.leave();
    h.settled();
    assert!(interval.0.lock().unwrap().watchers.is_empty());
}
//@show-end

//@show periods: waking before the deadline
#[guinea::test]
fn waking_before_a_deadline_does_not_create_an_immediate_tick(h: &mut Harness) {
    let interval = Interval::default();
    interval.set(Some(Duration::from_secs(1)));
    let page = h.child();
    page.install::<Polling>(&interval).unwrap();
    h.settled();
    h.advance(Duration::from_millis(250));
    page.scope().sleep();
    h.advance(Duration::from_millis(250));
    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(page.state::<Count>().0, 1);
}
//@show-end

//@show periods: removing the interval
#[guinea::test]
fn removing_the_period_value_disarms_the_current_wait(h: &mut Harness) {
    let interval = Interval::default();
    interval.set(Some(Duration::from_secs(1)));
    h.install::<Polling>(&interval).unwrap();
    h.settled();
    h.advance(Duration::from_millis(500));
    interval.set(None);
    h.advance(Duration::from_secs(10));
    assert_eq!(h.state::<Count>().0, 0);
    interval.set(Some(Duration::from_secs(1)));
    h.settled();
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Count>().0, 1);
}
//@show-end
