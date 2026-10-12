# Prior art: state, events and lifetimes

**Status: research, 2026-10-11.** Nothing here is decided. It surveys how
application-layer frameworks on other platforms handle shared state, events
and lifetimes, and maps what they learned onto six additions under
discussion for guinea:

1. `AsyncPush<R>`: a `Send` twin of `Push` that always hops to the UI
   thread; `Push` itself becomes `!Send`, the way `Cx` and `AsyncContext`
   are split.
2. `Latest<T>`: a ready-made "last value" reducer.
3. `follow::<R>(|&R| ...)`: snapshot-based, conflated listening that catches
   up on wake.
4. Production tied to awake followers, with a stop delay, after Kotlin's
   `stateIn(WhileSubscribed(..))`.
5. Declared event publishers: events listed in `Exports`, a subscription
   checked for a reachable publisher; the bus kept for facts and requests.
6. Derived state (selectors).

The motivating case is uniproc, which carries state over the global bus (the
latest agent report, connection state, machine sample) and so had to invent
a "subscribe, then ask for the current value" protocol (`AgentStateRequest`,
`Woke` on scope wake) and hand-written conflation (`building` + `pending` +
`epoch` in `ProcessesActor`).

Sources read locally: `docs/features.md`, `docs/regions.md`, `README.md`,
`guinea-core/src/scope/lifetime.rs`, uniproc's
`domain/src/features/processes/{install,actor}.rs`.

One finding up front, because it changes how the proposals read: in guinea a
sleeping scope is "kept, with its state and actors, but deaf" (`Scope::sleep`
doc). In Android and Decompose the **state holder keeps hearing** while the
UI is stopped - only the UI's *collection* pauses, and upstream production is
gated by counting those UI collectors. uniproc's catch-up protocol exists
because guinea deafens the state holder itself, not only its view. Every
proposal below should be read against that difference.

---

## 1. Per framework

### 1.1 Android Jetpack (Lifecycle, ViewModel, Navigation, architecture guide)

**(a) Lifetime and scoping**

- A ViewModel is scoped to a `ViewModelStoreOwner`: Activity, Fragment,
  `NavBackStackEntry`, a nav graph, or a composable via
  `rememberViewModelStoreOwner()`.
  - It lives while its owner is on the back stack; `onCleared()` runs when
    the owner is gone for good.
  - It survives configuration changes.
  - Graph-scoped ViewModels (`viewModel(navController.getBackStackEntry("parent"))`,
    `hiltViewModel(parentEntry)`) are the official way to share state across
    one flow.
    https://developer.android.com/topic/libraries/architecture/viewmodel/viewmodel-apis
- Lookup is explicit (you name the owner) or "closest owner" via
  `LocalViewModelStoreOwner` - close to guinea's nearest-ancestor resolve, but
  keyed by owner, not by type.
- The data-layer guide sorts work into three lifetimes.
  https://developer.android.com/topic/architecture/data-layer
  - **UI-oriented:** the caller's scope, cancelled when the user leaves.
  - **App-oriented:** an injected `externalScope` or Application scope.
  - **Business-oriented:** WorkManager, survives process death.
  - Rule: "Don't use the caller's CoroutineScope for app-oriented work."

**(b) Shared state**

- Single source of truth per repository. Repositories expose `Flow` and
  immutable data, and must be **main-safe**: they move their own work off the
  main thread.
- Background writes: `MutableStateFlow.value` / `update {}` is thread-safe
  (see 1.2).
  - LiveData splits writes: `setValue` is main-thread only; `postValue` hops
    from a worker, and if called several times before the main thread runs,
    "only the last value would be dispatched".
  - Pitfall: `postValue(a)` followed by a main-thread `setValue(b)` ends with
    **a** observed, because the queued post lands later.
    https://developer.android.com/reference/androidx/lifecycle/MutableLiveData
- Late subscribers: StateFlow, LiveData and `stateIn` always hand the current
  value on subscribe.
- Paused subscribers, LiveData:
  - It notifies only *active* observers (STARTED/RESUMED).
  - On reactivation the observer gets the latest value, and "if the observer
    changes from inactive to active a second time, it only receives an update
    if the value has changed since the last time it became active" - a
    version counter per observer.
    https://developer.android.com/topic/libraries/architecture/livedata
- Paused subscribers, Flow: `repeatOnLifecycle(STARTED)` /
  `collectAsStateWithLifecycle()` cancel the collector on STOP and restart it
  on START; StateFlow conflation then delivers just the latest value.
- Meanwhile the **ViewModel keeps collecting**: `viewModelScope` work "run[s]
  to completion, even when the UI isn't visible".
  https://developer.android.com/topic/architecture/ui-layer/state-production

**(c) Events vs state vs commands**

- Official guide: "ViewModel events should always result in a UI state
  update."
  - Channels and "other reactive streams" for one-off events are discouraged:
    "When the producer (the ViewModel) outlives the consumer … these
    solutions don't guarantee the delivery and processing of those events."
  - Transient messages become a state field the UI acknowledges
    (`userMessageShown()` clears it).
  - Navigation that needs validation becomes state the UI observes;
    UI-originated navigation is guarded with `dropUnlessResumed`.
  - "Multiple concurrent consumers" make exactly-once delivery "extremely
    hard".
  - https://developer.android.com/topic/architecture/ui-layer/events
- Background post (not fetched, Medium returned 403; content from memory):
  Manuel Vivo, "ViewModel: One-off event antipatterns" (2022).
  https://medium.com/androiddevelopers/viewmodel-one-off-event-antipatterns-16a1da869b95
- **Conflicting earlier guidance:** Roman Elizarov's "Shared flows, broadcast
  channels" (2020) recommended `Channel` + `receiveAsFlow()` for
  single-consumer events.
  https://elizarov.medium.com/shared-flows-broadcast-channels-899b675e805c
  - Channel delivery was later shown to lose elements on cancellation:
    https://github.com/Kotlin/kotlinx.coroutines/issues/2886
  - The Android guide's later stance supersedes the 2020 recommendation.
- There is no app-wide bus in Jetpack. App-level facts are a `Flow` /
  `SharedFlow` owned by a repository - a declared publisher.

**(d) Production tied to subscribers**

- LiveData `onActive()` / `onInactive()` fire on 0→1 and 1→0 *active*
  observers; the stock-price example connects in `onActive` and disconnects
  in `onInactive`. "The transformations aren't calculated unless an observer
  is watching."
- The `liveData {}` builder cancels its block a timeout (default 5 s) after
  the last active observer leaves, and restarts on reactivation. From
  knowledge; not on the pages fetched.
- Flow: `stateIn(scope, WhileSubscribed(5_000), initial)`. Official rationale:
  "keeps the subscription active for five seconds after the UI disappears to
  handle configuration changes."
  https://developer.android.com/topic/libraries/architecture/coroutines
- Guide alternative: `SharingStarted.Lazily` when the screen sits "on the back
  stack or in an offscreen tab". Production starts lazily, never in `init {}`.

**(e) Derived state**

- Flow: `combine(...)` then `stateIn` - the guide's default. LiveData:
  `map` / `switchMap` / `MediatorLiveData`, lazy and main-thread only.
- Compose: `derivedStateOf`, `snapshotFlow`.
- The guide moved the data layer *off* LiveData: "all LiveData objects
  (including ones created through transformations) are observed on the main
  thread", and its combination ability is "very limited".

**(f) Testing**

- Assert on `StateFlow.value`, not on the emission sequence: conflation makes
  intermediate values unobservable.
- `stateIn(WhileSubscribed | Lazily)` needs a collector in the test, or "the
  StateFlow's value will never be updated". Start one in `backgroundScope`
  with `UnconfinedTestDispatcher`; Turbine covers sequences.
  https://developer.android.com/kotlin/flow/test

### 1.2 Kotlin coroutines Flow

**(a) Lifetime**

- The `CoroutineScope` a hot flow is started in (`viewModelScope`,
  `externalScope`). `shareIn` / `stateIn` require a scope that "should outlive
  any consumer".

**(b) Shared state: StateFlow**

- Always has a value. "Updates to the value are always conflated." Equal
  values are suppressed via `equals`, like `distinctUntilChanged`; behaviour
  with a broken `equals` is "unspecified".
- "All methods of state flow are thread-safe"; `update {}` is an atomic
  read-modify-write. It never completes.
- Equivalent to `MutableSharedFlow(replay = 1, onBufferOverflow = DROP_OLDEST)`
  plus `distinctUntilChanged`.
  https://kotlinlang.org/api/kotlinx.coroutines/kotlinx-coroutines-core/kotlinx.coroutines.flow/-state-flow/
- SharedFlow offers `replay`, `onBufferOverflow` (SUSPEND / DROP_LATEST /
  DROP_OLDEST), `subscriptionCount` (a StateFlow of the collector count) and
  `resetReplayCache()`.
  https://developer.android.com/kotlin/flow/stateflow-and-sharedflow
- Late and paused subscribers get the replay cache (1 for StateFlow). There
  is no per-subscriber "changed since last seen" check; LiveData has one.

**(c) Three primitives for three kinds of traffic**

- StateFlow is state. SharedFlow with `replay = 0` is broadcast, lost when
  nobody subscribes. Channel is a queue: each element goes to *one* receiver
  and is buffered while none exists.

**(d) `SharingStarted`**

- `Eagerly` starts now and never stops. `Lazily` starts on the first
  subscriber and never stops.
- `WhileSubscribed(stopTimeoutMillis = 0, replayExpirationMillis = Long.MAX_VALUE)`:
  - stops upstream `stopTimeout` after the last subscriber leaves;
  - resets the replay cache `replayExpiration` after the stop - back to
    `initialValue` for `stateIn`, an empty cache for `shareIn`;
  - a subscriber arriving within the timeout cancels the stop; one arriving
    after the stop restarts upstream. The restart behaviour is library
    semantics, not stated on the KDoc page.
  - https://kotlinlang.org/api/kotlinx.coroutines/kotlinx-coroutines-core/kotlinx.coroutines.flow/-sharing-started/-companion/-while-subscribed.html
- `SharingStarted` is an interface: a policy maps
  `subscriptionCount: StateFlow<Int>` to START / STOP /
  STOP_AND_RESET_REPLAY_CACHE, so custom policies are first-class. From
  knowledge; not re-fetched.
- Lifecycle gating on the consumer side: `repeatOnLifecycle` /
  `flowWithLifecycle` / `collectAsStateWithLifecycle` (lifecycle-runtime-ktx
  ≥ 2.4) cancel the collector below the target state, which lowers
  `subscriptionCount`, which stops a `WhileSubscribed` upstream.
  - `launchWhenStarted` was deprecated because it *suspends* rather than
    cancels, so upstream keeps running. From knowledge; the official page
    only says to use `repeatOnLifecycle`.

**(e) Derived state**

- `map` / `combine` return a cold `Flow`, not a `StateFlow`; getting a value
  back takes `stateIn` with a **scope**.
- A `StateFlow.map → StateFlow` operator has been requested for years and not
  added, because a derived hot value needs an owner to collect upstream:
  https://github.com/Kotlin/kotlinx.coroutines/issues/2081 (and #2631 for
  `combineStates`).
- Conflation operators: `conflate()` (finish the current item, then take the
  latest) and `collectLatest` / `mapLatest` (cancel in-flight work when a
  newer value arrives).

**(f) Testing:** as in 1.1, plus virtual time in `runTest`.

### 1.3 Decompose (arkivanov), 3.x

**(a) Lifetime**

- Every component gets a `ComponentContext`: an Essenty `Lifecycle`,
  `StateKeeper` (survives configuration change and process death),
  `InstanceKeeper` (ViewModel-like retained objects; `retainedInstance {}`
  stable since 3.2.0) and `BackHandler`.
  https://arkivanov.github.io/Decompose/component/overview/ ,
  https://arkivanov.github.io/Decompose/component/instance-retaining/
- Child stack:
  - pushing stops the previous child, popping resumes it;
  - "components in the back stack are always stopped", yet they "can still
    run business logic while stopped";
  - destruction runs in reverse order, active first.
  - https://arkivanov.github.io/Decompose/navigation/stack/overview/
- Child pages: the selected page is ACTIVE, its two neighbours INACTIVE
  (alive but stopped), the rest DESTROYED; custom policies allowed.
  https://arkivanov.github.io/Decompose/navigation/pages/overview/
- Children cannot outlive their parent. A permanent child is made with
  `childContext(key)` and destroyed with its parent.
  https://arkivanov.github.io/Decompose/component/child-components/
- **Desktop:** `LifecycleController(registry, windowState, windowInfo?)`:
  - window minimized → `stop()` (CREATED);
  - focused → RESUMED;
  - visible but unfocused → STARTED.
  - https://github.com/arkivanov/Decompose/blob/master/extensions-compose/src/jvmMain/kotlin/com/arkivanov/decompose/extensions/compose/lifecycle/LifecycleController.kt

**(b) Shared state**

- No ambient lookup. "Each parent component is only aware of its immediate
  children"; shared things are passed through constructors, and for a reused
  subtree "you only need to satisfy its top component's dependencies".
- `Value` / `MutableValue` is thread-safe, but subscribe and update "should
  happen only on the main thread". StateFlow can be used instead.
- No built-in catch-up beyond `Value` handing the current value on subscribe.

**(c) Events, state, commands:** not prescribed. Children report upward
through callbacks or an output passed into the constructor.

**(d) Production tied to subscribers:** none built in. Gating comes from the
component lifecycle (`doOnStart` / `doOnStop`, Essenty `coroutineScope()`).

**(e) Derived state:** `Value.map` (exact semantics not verified), or Flow.

**(f) Testing:** components are plain classes; tests drive a
`LifecycleRegistry` by hand (`resume()`, `stop()`, `destroy()`).

### 1.4 MVIKotlin (arkivanov), v4

The closest analogue to guinea. https://arkivanov.github.io/MVIKotlin/store/

**(c) The Store model**

- `Store<Intent, State, Label>`.
- The Executor (≈ actor) handles Intents and Actions, calls `dispatch(Msg)`
  into a pure Reducer (`Msg` ≈ guinea's `Update`), and calls `publish(Label)`
  for one-off outputs.
- A Bootstrapper produces Actions at init (≈ `emit(Refresh)` in `install`).
- **Threading rule:** `accept` / `init` / `dispose`, the Executor and the
  Reducer all run on the main thread. "The Executor and Bootstrapper may
  switch threads for async work, but must dispatch Messages, Actions, and
  Labels back on the main thread." The new state is visible to the Executor
  right after `dispatch`.
- Labels are "one-time events": not cached, delivered to current subscribers
  only. A Label published during init may have no subscribers; the remedy is
  `autoInit = false`.
- The maintainer's advice (Kotlin Slack, secondhand): Labels "can be missed"
  while in background, so navigate via state; Labels exist for when "state
  isn't suitable".
- v4 added `labelsChannel(scope, capacity = BUFFERED)` "when labels must not
  be skipped while there is no subscriber", recommending "one Channel per
  subscriber". Source: `mvikotlin-extensions-coroutines/.../StoreExt.kt` on
  GitHub master.
- **Labels are declared in the Store's type.** You subscribe to *this
  store's* labels, never to a global label type.

**(a) Lifetime:** `dispose()` cancels async work; typically
`lifecycle.doOnDestroy(store::dispose)`, or the store is kept in an
InstanceKeeper.

**(b) Shared state:** `stateFlow(scope, started = Eagerly)` is a hot shared
view of `states`; subscribers immediately get the current State.

**(d) Production tied to subscribers:** `Binder` with
`BinderLifecycleMode.START_STOP` connects state → view and labels → consumer
only while started. Nothing counts subscribers.
https://arkivanov.github.io/MVIKotlin/binding_and_lifecycle/

**(e) Derived state:** the State → Model mapper on the view side.

**(f) Testing:** `TimeTravelStoreFactory` and `LoggingStoreFactory`
wrappers, plus plain Executor / Reducer unit tests.

### 1.5 Prism (WPF), 9.x

**(a) Lifetime**

- Modules (`IModule.RegisterTypes` / `OnInitialized`) are loaded and
  initialized **once** for the application's lifetime. No documented unload;
  no source found either way.
- Region navigation:
  - `INavigationAware.OnNavigatedTo` / `IsNavigationTarget` (reuse the
    existing view) / `OnNavigatedFrom` ("save any state");
  - `IConfirmNavigationRequest` can veto;
  - `IRegionMemberLifetime.KeepAlive` / `[RegionMemberLifetime]`: when false,
    the view is removed from the region on deactivation and becomes
    collectable.
  - https://docs.prismlibrary.com/docs/9.0/navigation/regions/view-viewmodel-participation ,
    https://docs.prismlibrary.com/docs/9.0/navigation/regions/controlling-view-lifetime
- `RegionContext` shares an object from a region host to the views inside it
  - the closest thing to guinea's layout exports.

**(b) Shared state:** "shared services" registered in the container (Stock
Trader's `IMarketFeedService`). No observable-state primitive; services raise
.NET events or INPC.

**(c) Four channels, with explicit guidance** (Prism 5 guide, ch. 9).
https://learn.microsoft.com/en-us/previous-versions/msp-n-p/ff921122(v=pandp.40)

- Commanding (`DelegateCommand`; `CompositeCommand` for a global Save All with
  aggregated `CanExecute`): "expectation of immediate action".
- Region context: host ↔ view context.
- Shared services: "if none of the preceding is applicable".
- EventAggregator: "when there is not a direct action-reaction expectation".

**EventAggregator (`PubSubEvent<T>`)**

- Typed. The event is built on the first `GetEvent<T>()`, so **no publisher
  is declared or checked**. No replay.
- `ThreadOption`: PublisherThread (default), UIThread, BackgroundThread.
  UIThread requires the aggregator to be "initially constructed on the UI
  thread": it captures the UI context at construction.
- `keepSubscriberReferenceAlive = false` (the default) holds a weak reference
  and unsubscribes on GC. It is slower; strong references with a manual
  unsubscribe are recommended for high event rates.
- `filter: Predicate<T>`; subscribing returns a `SubscriptionToken`.
- Payloads should be immutable, since several threads may read them.
  https://docs.prismlibrary.com/docs/9.0/event-aggregator

**Criticism** (thin, mostly secondhand):

- The API shape: you fetch an "event" that is really a channel.
- Type-only routing, no topics.
  https://learn.microsoft.com/en-us/archive/blogs/ukadc/a-suck-less-event-aggregator-for-prism
- A hidden communication graph.
- Weak references plus lambda subscriptions silently stop firing once the
  closure is collected (community reports).
- No canonical "EA is an antipattern" source was found; Jeremy Miller's
  critique is cited secondhand only.

**(d) Production tied to subscribers:** none. **(e) Derived state:** none
(INPC). **(f) Testing:** mock `IEventAggregator`.

### 1.6 Swift TCA (brief)

Only what it adds beyond the others.
https://pointfreeco.github.io/swift-composable-architecture/ ,
https://pointfree.co/blog/posts/135-shared-state-in-the-composable-architecture

- `Scope` embeds child reducers.
- `Effect.run` / `.cancellable(id:, cancelInFlight: true)`: **latest-wins
  cancellation by id** as a primitive.
- `@Dependency` with live, test and preview values, overridden per test.
- `@Shared(.inMemory / .appStorage / .fileStorage)` (1.10+): a
  reference-semantics shared value; `$shared.withLock {}` (1.11) is required
  for async mutation.
- `TestStore` is **exhaustive**: every state change, including in `@Shared`,
  and every received action must be asserted, and in-flight effects at the
  end of a test fail it. `withExhaustivity(.off)` relaxes this.
- Derived state is plain computed properties on State.

### 1.7 ReactiveUI (brief)

- `WhenActivated` ties subscriptions to view activation and disposes them on
  deactivation; it matters for subscriptions to longer-lived objects.
- `ObservableAsPropertyHelper` / `ToProperty`: a **derived, read-only
  property** fed from an observable, with an explicit `scheduler` (by default
  "whatever the current thread's scheduler is", so UI code passes the
  main-thread scheduler). `deferSubscription: true` subscribes on the first
  read of `Value` - lazy production.
- The docs suggest `ReplaySubject(1)` for hot sources.
- https://reactiveui.net/documentation/handbook/observable-as-property-helper/ ,
  https://reactiveui.net/api/reactiveui.oaphcreationhelpermixin ,
  https://reactiveui.net/documentation/guidelines/framework/prefer-oaph-over-properties/

---

## 2. Comparison

| | (a) lifetime vs navigation | (b) shared state / bg writes / paused catch-up | (c) events vs state | (d) production by subscribers | (e) derived |
|---|---|---|---|---|---|
| **Jetpack** | VM per owner (entry, graph, activity, composable); back-stack entries alive; three-tier operation lifetime | repository SSOT; LiveData `set` (main) / `post` (bg, conflating); active-only delivery, latest on reactivation, version-checked | events → state; one-off streams discouraged; no global bus | LiveData `onActive` / `onInactive`; `liveData {}` 5 s timeout | `map` / `switchMap` / Mediator (lazy, main-only); `combine` + `stateIn` |
| **Flow** | a scope that outlives consumers | StateFlow thread-safe, conflated, equals-suppressed; latest on (re)subscribe | StateFlow / SharedFlow / Channel as separate primitives | `WhileSubscribed(stop, replayExpiry)`; custom `SharingStarted` over `subscriptionCount` | cold operators; a hot derived value needs a scope (#2081) |
| **Decompose** | component tree mirrors navigation; back stack alive and stopped; pages ACTIVE / INACTIVE / DESTROYED; desktop minimize → stopped | passed through constructors; `Value` main-thread; current value on subscribe | unprescribed; callbacks upward | none (lifecycle hooks only) | `Value` / Flow mapping |
| **MVIKotlin** | Store disposed with the lifecycle | `stateFlow(scope)`; dispatch must be on main | State vs **Labels declared per Store**; `labelsChannel` for no-loss | Binder START_STOP | view-side mapper |
| **Prism** | modules for the app's lifetime; region views `KeepAlive` | shared services; no observable primitive | commands / region context / services / EA; EA has no replay and no declared publisher | none | none |
| **TCA** | `Scope`; effects cancelled by id | `@Shared` + `withLock` | actions only; no bus | none (view `.task`) | computed properties |
| **ReactiveUI** | `WhenActivated` | OAPH with a scheduler | observables | `deferSubscription` | OAPH, `WhenAnyValue` |

---

## 3. Mapping to guinea

### Proposal 1. `AsyncPush<R>` (Send, always hops); `Push` becomes `!Send`

**Prior art**

- LiveData `setValue` / `postValue` is exactly this split.
- MVIKotlin and Decompose instead *forbid* off-main dispatch and make the
  executor hop back itself.
- Prism `ThreadOption.UIThread` captures the UI context when the aggregator
  is constructed.
- Kotlin took the other road, a thread-safe holder with a lock. That is not
  open to guinea: the scope tree is `thread_local`.

**Pitfalls**

1. **Conflation must not apply to deltas.** `postValue` conflates because it
   carries a *value*. guinea's `Push` carries an `Update`, a delta, and
   dropping deltas corrupts state. `AsyncPush` must either deliver every
   update in order, or conflate only for replace-style reducers
   (`Latest<T>`), where conflating is correct.
2. **Ordering inversion.** LiveData documents that a background post
   followed by a main-thread set ends with the background value. With
   `AsyncPush` and `Push` writing the same reducer, a hop queued earlier can
   land after a later direct push. Document it, or route both through one
   queue.
3. **Late arrival.** Background work finishing after its scope left is
   inherent, not a bug. guinea makes "push to a page that left" a caller
   error (panic). That is right for `Push` and wrong for `AsyncPush`: Kotlin
   cancels the scope so the continuation never resumes, and LiveData simply
   has no active observer. An `AsyncPush` arriving at a removed scope should
   be dropped, and traced, not panic.
4. **Capture the dispatcher at construction**, on the UI thread, as Prism
   does. `AsyncPush` is then obtainable only from a UI-side context.

**Against:** if most off-thread producers are `cx.task()` (already
scope-owned, with results hopping back to the actor), `AsyncPush` is needed
only for foreign threads such as the agent connection. Count the call sites
first; MVIKotlin lives without it.

### Proposal 2. `Latest<T>`

**Prior art:** universal - StateFlow, LiveData, Decompose `Value`,
`BehaviorSubject`, `@Shared`.

**Semantics they chose**

- An initial value is required (StateFlow) or nullable (LiveData). guinea's
  `Load<T>` covers "not yet".
- **Equal values are suppressed** (StateFlow `equals`). `Latest<T>` has to
  decide between `PartialEq` and `Rc::ptr_eq`. At hundreds of rows per
  second, `ptr_eq` on an `Rc` is the cheap, correct default; a deep
  `PartialEq` on a row vector every tick is not.
- `resetReplayCache` / `replayExpiration` exist because a stale value on
  return is sometimes wrong. That is a product decision per source, not a
  framework default.

**Against:** nothing serious. Keep it a plain reducer with `Update = T`, so it
composes with everything else.

### Proposal 3. `follow::<R>(|&R| ...)` (snapshot, conflated, catches up on wake)

**Prior art:** LiveData `observe` is the precise precedent; also
`collectAsStateWithLifecycle` / `repeatOnLifecycle` over StateFlow, and
Decompose `Value.subscribe`.

**Semantics to copy**

- Deliver the current snapshot immediately on subscribe (everyone does).
- Skip intermediate values while asleep.
- On wake, deliver **only if it changed since the last delivery to this
  follower**. LiveData keeps a version per observer; `repeatOnLifecycle` does
  not and reruns the block. Use a per-follower version, or `ptr_eq`.

**Pitfalls**

- Not for facts or events: the Android guide shows conflated delivery loses
  them.
- Restart semantics: `repeatOnLifecycle` reruns the whole block on every
  START, so side effects in it repeat. A `follow` body should be a pure
  reaction to the snapshot.
- Ordering against `observe`: decide whether `follow` fires before or after
  delta observers in the same drain.

**What it replaces in uniproc:** `Woke` and `AgentStateRequest` disappear if
the agent's report is a `Latest<WindowsReport>` owned *above*
`ProcessesFeature`, in the app scope. The sleeping feature follows it and
gets the latest snapshot on wake. This is the Android shape: the source keeps
hearing, only the consumer sleeps.

**Against:** with `observe` for the delta and `read` for the snapshot, a
third listening verb adds surface. The justification is that catch-up on wake
for actors, which are not render code, is otherwise reinvented per feature,
as uniproc shows.

### Proposal 4. Production tied to awake followers, with a delay

**Prior art**

- Kotlin `WhileSubscribed(stopTimeout, replayExpiration)`; custom
  `SharingStarted` over `subscriptionCount`.
- LiveData `onActive` / `onInactive`, and `liveData {}` with its 5 s.
- TanStack Query: `refetchInterval` runs while observers exist, and
  **`gcTime` (default 5 min) governs dropping cached data, not stopping
  production**. Two separate knobs - Kotlin's two parameters keep them
  separate too.
  https://tanstack.com/query/latest/docs/framework/react/guides/important-defaults

**Semantics to copy**

- **Count only active (awake) followers**, as LiveData does. In Kotlin this
  falls out of lifecycle-gated collectors cancelling themselves.
- **Two independent delays:** stop producing, and forget the value.
- The policy is a function of the follower count, so Eagerly / Lazily /
  WhileFollowed are three policies of one mechanism.

**Pitfalls**

1. **The 5000 ms does not transfer.** Android's stated reason is
   configuration change (the activity torn down and recreated); a Slack
   answer also ties it to the 5 s ANR threshold. A desktop has no
   configuration change. guinea's analogues are route transitions and
   keep-layout sleep / wake; pick the delay from those, not 5 s by habit.
2. **Who counts as a follower.** The tray icon and tooltip, notifications and
   background history recording are non-UI consumers. If they are not
   followers, production stops silently when the window hides. Each of them
   needs a way to hold a follow.
3. **Restart cost.** Stopping an agent connection is not free: a restart is
   a reconnect. The stop timeout is the hysteresis, and some sources should
   simply be Eagerly.
4. **Tests need a follower.** The Android flow-testing guide calls this out;
   guinea's harness should make "an awake follower" trivial to create.
5. **Owner scope.** `stateIn`'s scope "should outlive any consumer". The
   producing feature must sit above every follower, which guinea's resolve
   already enforces.

**Against:** scope sleep and removal already stop the timers and
subscriptions owned by the *consumer's* scope. This proposal adds gating for
a source owned *above* its consumers, needed only where the source is shared
and expensive (the agent feed, the machine sampler). It should not be the
default for every reducer.

### Proposal 5. Declared event publishers

**Prior art for:** MVIKotlin Labels are declared in the Store's type, and
you subscribe to *that store's* labels. Android has no global bus; app-wide
facts are a repository-owned SharedFlow, a declared publisher by
construction. TCA has no bus at all.

**Prior art against the status quo:** Prism's EventAggregator builds the
event on first access, so no publisher is ever checked. Its documented and
community pain points are exactly that: a hidden graph, silent non-delivery
(weak lambda references), no topics.

**Pitfalls**

- Declared events are still lost when nobody is subscribed or the subscriber
  sleeps. MVIKotlin added `labelsChannel` (buffered, "one Channel per
  subscriber") for no-loss delivery, and still recommends state for
  navigation.
- So events stay for facts nobody needs to have seen. Anything a late or
  sleeping subscriber must know is state.
- RPC with exactly one answerer has no direct analogue: Android uses suspend
  functions on a repository, Prism uses shared services. guinea's "exactly
  one answerer" check is stricter than any of them.

### Proposal 6. Derived state (selectors)

**Prior art**

- Flow `combine` + `stateIn`.
- LiveData `map` / `switchMap` / `MediatorLiveData`, lazy, not computed
  "unless an observer is watching".
- Compose `derivedStateOf`.
- ReactiveUI OAPH with `deferSubscription`.
- TCA computed properties.

**The lesson of kotlinx.coroutines #2081:** a *hot, cached* derived value
needs an owner scope; Kotlin refused a scope-less `StateFlow.map`.

**Two tiers fit guinea**

1. **Pure functions over snapshots**, computed at read time and memoized on
   the identity of the input `Rc`s. Needs no home and covers most cases -
   TCA's choice.
2. **A derived reducer installed in a scope** below all its inputs and above
   all its readers - guinea's resolve rule applied to a computed value,
   recomputed by `follow`ing its inputs.

**Pitfalls**

- LiveData's main-thread-only derivation pushed Android back to Flow for
  heavy derivations.
- Expensive derivation (rows from a report) must run off-thread with
  latest-wins (see gap 1 below).

**Against:** tier 2 is a reducer plus a `follow` and needs no new mechanism
once proposal 3 exists. Ship tier 1 as documentation or a helper first.

### Gaps outside the six, judged for a desktop

1. ~~**Latest-wins background computation.**~~ Answered by `cx.task()`:
   uniproc's `building` + `pending` + `epoch` was a hand-written
   `.conflate()` plus invalidation (Kotlin `mapLatest` / `collectLatest`,
   TCA `.cancellable(id:, cancelInFlight:)`). A lane per actor and answer,
   or per `.lane::<K>()` and `.keyed(k)`, takes `latest`, `conflate`,
   `queue` or `exhaust` - RxJS's `switchMap`, conflation, `concatMap` and
   `exhaustMap` - and drops a stale answer by generation where it is
   delivered.
2. **Window visibility as a cause of sleep.** Decompose maps minimized to
   stopped, visible-unfocused to started, focused to resumed. guinea appears
   to sleep scopes only for route reasons (keep-layouts); `windows.rs` tracks
   `minimized` only for geometry, though not every path was traced. A window
   minimized or hidden to the tray could put its scopes to sleep, which with
   proposal 4 stops scanning when nobody looks.
   - Do **not** copy RESUMED (focused) as a gating level: several visible
     windows must all keep updating.
3. **A taxonomy of operation lifetimes** (UI / app / business). The agent
   link and report are app-oriented and belong in features installed by
   `app!`. The open question in regions.md about segment `Feature` vs
   `AppFeature` blocks putting them there cleanly - the same issue seen from
   the state side.
4. **Exhaustive tests** (TCA's `TestStore`): assert every update and fail on
   background work still in flight at the end of a test. It would complement
   guinea's seeded determinism.
5. ~~**Dependency overrides for tests.**~~ Already answered: a test hands
   its own value to `Harness::provide`, and the feature reads it with
   `require_or_default`, so uniproc's `ProcessesDeps` is a TCA `@Dependency`
   built on the framework's own mechanism, not a hand-rolled one.
6. **A guard on UI-originated navigation** (`dropUnlessResumed`): ignore a
   navigation fired from a scope that is no longer active, such as a double
   click during a transition. guinea's routing guards may already cover it;
   not checked.
7. **Equality suppression on `observe` / `follow` / re-render** (StateFlow
   `equals`, TanStack structural sharing). At hundreds of rows per second,
   identity-preserving row updates - an unchanged row keeps its `Rc` - let
   the table diff by pointer.

**What does not transfer from mobile:** configuration changes and the 5 s
rationale; process death and `SavedStateHandle` / `StateKeeper` (guinea's
`restorable` covers session restore differently); the ANR threshold;
"consume UI messages only in the foreground" (a tray application notifies
while hidden); RESUMED as "the one screen the user is on";
`Dispatchers.Main.immediate` subtleties; battery-driven backgrounding (CPU
cost still matters).

---

## 4. Uncertain or conflicting

- **Medium posts** (Vivo 2022, Elizarov 2020, Alcérreca on
  `WhileSubscribed(5000)`) returned 403; their content above is from memory
  and secondhand summaries. The official Android sentence on the 5 s ("to
  handle configuration changes") *was* verified.
- **`WhileSubscribed` restart behaviour** - a resubscribe within the timeout
  cancels the stop; after the stop upstream restarts with the cache kept
  until expiry - is library semantics, not spelled out on the KDoc page.
- **The `liveData {}` 5 s timeout** and **the reason `launchWhenX` was
  deprecated** are from knowledge, not fetched pages.
- **Prism module unload:** no source either way; load-once by design.
- **Criticism of Prism's EventAggregator** is sparse and blog-level, with no
  canonical reference.
- **Decompose `Value.map` semantics** were not verified.
- **Elizarov (2020: Channel for events) vs the Android guide (later: reduce
  events to state)** is a real conflict; the Android stance is the current
  official one.
- **Whether guinea already sleeps scopes on minimize, and whether its routing
  guards cover navigation from a stale scope**, was not traced through all
  the code.
