---
title: Changing timer periods
sidebar:
  order: 12
---

A fixed timer accepts `Duration`. `Period::follows(source, map)` instead
reads its period from an observable value. Changing the value interrupts
the current wait and starts a full new period; it does not preserve the
remaining fraction of the old wait. No value means no tick.

## A period stored in amethystate

For application settings, use
[`guinea-plugin-store`](https://github.com/guinea-rs/guinea-plugins/tree/master/plugins/store).
It opens and closes the store. Its `StoreAccess` extension trait provides
`settings::<S>()` and `try_settings::<S>()` for opening declared settings
over that store, rather than constructing individual fields or using
amethystate's global store.

The current plugin still targets guinea's previous application API.
Its context extension is not yet compatible with the API this book
documents. Until that integration is updated, the executable example below
uses amethystate's public `new_with(&store)` constructor. It does not pretend
to demonstrate a working plugin installation.

### Declare the setting

Suppose the user chooses how often a process list refreshes. Declare the
interval in milliseconds as part of the settings struct:

<!-- shown: periods: declaring refresh settings -->
```rust
use amethystate::{Field, amethystate};

#[amethystate(prefix = "refresh")]
pub struct RefreshSettings {
    #[amestate(default = 1000u64)]
    pub interval_ms: u64,
}
```
<!-- /shown -->

`settings.interval_ms()` returns the field handle. No manual field path,
UUID or test utility is needed. Pass that handle to the feature:

<!-- shown: periods: installing an amethystate timer -->
```rust
feature! {
    pub ConfiguredPolling {
        exports { Count }
    }
}

#[installs]
fn configured_polling(cx: &Install<Field<u64>>) -> anyhow::Result<ConfiguredPolling> {
    let count = cx.state::<Count>().plain();
    let period = Period::follows(Milliseconds(cx.params().clone()), Duration::from_millis);
    cx.repeat(period, move || count.push(()));
    Ok(ConfiguredPolling(count))
}
```
<!-- /shown -->

`Count` is the reducer defined below. A write through the settings accessor
restarts the timer's wait; the feature does not need reinstalling.

### Check the interval change

The test opens an in-memory store with the public `StoreBuilder` API,
opens `RefreshSettings` and uses its generated accessor. This example uses
amethystate 0.27 with the `memory` feature:

<!-- shown: periods: changing an amethystate field rearms the timer -->
```rust
#[guinea::test]
fn changing_an_amethystate_field_restarts_the_timer(h: &mut Harness) {
    let store = amethystate::StoreBuilder::in_memory().build().unwrap();
    let settings = RefreshSettings::new_with(&store);
    let interval = settings.interval_ms();
    h.install::<ConfiguredPolling>(&interval).unwrap();
    h.settled();
    h.advance(Duration::from_millis(500));

    interval.set(2000);
    h.settled();
    h.advance(Duration::from_millis(1500));
    assert_eq!(h.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 1);
    assert_eq!(interval.get(), 2000);
}
```
<!-- /shown -->

The field changes halfway through the initial second. No tick arrives at
the old deadline: the timer waits two full seconds after the change.
The test checks this timing, not persistence to disk.

### Bridge to the current timer API

The example implements `guinea::sources::Changing` directly for an
amethystate field. This local bridge shows how to read the current value
and keep the change subscription alive:

<!-- shown: periods: an amethystate field adapter -->
```rust
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
```
<!-- /shown -->

`now` reads the field's value, which always exists. `watch` returns an
amethystate subscription guard; the timer owns it and dropping it ends the
subscription. The remaining imports are `std::any::Any`,
`std::time::Duration`, `guinea::sources::Changing` and `guinea::prelude::*`.

## Implementing another observable setting

The source implements `guinea::sources::Changing`: read its current value
and register a thread-safe change callback. What `watch` returns must remove
that subscription when dropped. `Period` holds it for the timer's lifetime.

Here is a small in-memory setting, independent of any store library:

<!-- shown: periods: an observable interval -->
```rust
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
```
<!-- /shown -->

The imports are `std::any::Any`, `std::collections::BTreeMap`,
`std::sync::{Arc, Mutex}`, `std::time::Duration` and
`guinea::sources::Changing`. Callbacks are cloned out of the lock before
notification, so invoking them does not require holding the setting's mutex.
The returned `Unwatch` guard removes exactly its own registration.

If a reactive library already has a guinea adapter, use its `Changing`
implementation rather than writing one for the same type again.

## Installing the timer

<!-- shown: periods: installing a dynamic timer -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

#[reducer]
fn count(this: &mut Count, _: ()) {
    this.0 += 1;
}

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
```
<!-- /shown -->

This uses `guinea::prelude::*`. `Interval` stores `Duration` already, so
`identity` is enough. A numeric setting could use `Duration::from_millis`
as the mapping function instead.

The period is not a public enum to destructure. Construct it with
`Period::follows`, or pass a fixed `Duration` where `Into<Period>` is accepted.

<!-- shown: periods: a change restarts the wait -->
```rust
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
```
<!-- /shown -->

Initially there is no value, so even ten seconds produce no tick. Setting
one second starts the first wait; changing it halfway through to two seconds
restarts that wait. The next tick is two seconds after the change, not the
original deadline and not an immediate tick.

Removing the value also disarms an already-started wait; restoring it starts
a new one:

<!-- shown: periods: removing the interval -->
```rust
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
```
<!-- /shown -->

## Sleep and changes

Changing the setting does not wake a sleeping consumer into receiving
callbacks. An already-started pull can become overdue while asleep. On wake,
one held tick can arrive, followed by waits using the current period:

<!-- shown: periods: changes while asleep -->
```rust
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
```
<!-- /shown -->

This test starts a wait before sleeping, changes the period, and sleeps
long enough to make that wait overdue. It gets one tick, not all ten missed
half-second intervals. This is not a promise that every wake produces a tick:
if no wait is overdue, it still has to finish. Removing the page cancels
the source and drops its watcher. The setting itself may live longer.

Waking before the deadline does not manufacture a tick:

<!-- shown: periods: waking before the deadline -->
```rust
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
```
<!-- /shown -->
