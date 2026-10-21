---
title: Changing timer periods
sidebar:
  order: 12
---

Suppose an application refreshes its process list once a second, and the
user changes that interval in settings. The timer should pick up the new
value without reinstalling the feature.

Instead of a fixed `Duration`, pass `Period::follows(source, map)` to
`cx.repeat`. The source reports changes; the mapping function converts
its value into a duration.

## A setting in amethystate

Declare the interval in milliseconds. The generated `interval_ms()` accessor
returns a field handle for both reading and changing the setting.

<!-- shown: periods: declaring refresh settings -->
```rust
use amethystate::{Field, Store, amethystate};

#[amethystate(prefix = "refresh")]
pub struct RefreshSettings {
    #[amestate(default = 1000u64)]
    pub interval_ms: u64,
}
```
<!-- /shown -->

For this example, each tick increments a counter. An application could
instead send an actor a message requesting a process-list refresh.

<!-- shown: periods: counting refreshes -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

#[reducer]
fn count(this: &mut Count, _: ()) {
    this.0 += 1;
}
```
<!-- /shown -->

## The feature receives the store

The feature depends on `Store`, not on an individual settings field. Its
installer declares that dependency as an argument; guinea supplies the
store from an installed provider. The feature opens its own settings and
creates the timer:

<!-- shown: periods: installing an amethystate timer -->
```rust
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
```
<!-- /shown -->

`Duration::from_millis` converts the field value into a period.
`Milliseconds` connects the amethystate subscription to the timer; its
implementation appears below. `try_new_with` propagates a settings-opening
error through `?`. A missing store is different: it is a dependency wiring
error, and guinea panics before entering the installer.

An application can get its store from
[`guinea-plugin-store`](https://github.com/guinea-rs/guinea-plugins/tree/master/plugins/store).
The feature needs neither a context service lookup nor `cx.settings()`:
it receives `Store` as an argument. The plugin and guinea versions must be
compatible.

For this chapter's executable example, a small plugin opens a real
amethystate store in memory. It does not persist settings to disk or
replace the application's storage configuration:

<!-- shown: periods: a store for the example -->
```rust
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
```
<!-- /shown -->

`provides { Store }` declares the plugin's output. After a successful build,
guinea automatically makes that output available to installers.

## What a change does

The test uses the same store for the plugin and feature. It changes the
setting through the generated accessor, as a UI event handler would:

<!-- shown: periods: changing an amethystate field rearms the timer -->
```rust
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
```
<!-- /shown -->

The initial period is one second. Halfway through that second, the test
changes it to two seconds. This cancels the current wait: the next tick
arrives **two full seconds after the change**, not at the old deadline or
immediately. Subsequent ticks use the new period. `Harness` checks the
timing without waiting in real time or testing disk persistence.

## Subscribing to the field

The timer needs a `guinea::sources::Changing` source. This example uses a
local wrapper around `Field<u64>` to read its value and subscribe to changes:

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

`watch` returns an amethystate subscription guard. The timer keeps it;
removing the timer drops the guard and unsubscribes. A compatible
`amethystate-guinea` adapter already offers `IntoChanging::changing()`,
so an application using that adapter does not need this wrapper.

The remaining imports are `std::any::Any`, `std::time::Duration`,
`guinea::prelude::*` and `guinea::sources::Changing`; the test also needs
`guinea::app::Harness`. The example is tested with amethystate 0.27 and
its `memory` feature.

## Behaviour boundaries

- If `Changing::now()` returns `None`, there are no ticks. Removing the
  value interrupts the current wait; restoring it starts a new one.
  The `u64` field in this example always has a value.
- A period change does not wake a sleeping scope. If an existing wait
  finishes during sleep, one held tick may arrive on wake, not a queue
  of all missed ticks. Waking before the deadline does not create a tick.
- Removing the scope cancels the timer and drops its subscription.
  The store itself may outlive that scope.
