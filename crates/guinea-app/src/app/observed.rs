use guinea_core::actor::snapshot::ActorSnapshot;
use guinea_core::inspect::{Node, Open};
use guinea_core::observability::observer::{self, Describer, Event, Watching};
use guinea_core::observability::panels::{self, Panel};
use guinea_core::remote;
use guinea_core::scope::{DescribedState, Scope, ScopeId};

use super::registry::{Backend, Plugins};
use super::roots::{self, RootId};

/// The application, as the one tool watching it sees it: devtools.
///
/// What is in the application and what moves in it comes through
/// [`watch`](Self::watch), a batch at a time; a state's or an actor's value
/// is read here when the tool asks for it. Nothing else reads the
/// application from outside.
///
/// ```ignore
/// #[builds]
/// fn devtools(app: &mut PluginBuilder, params: DevToolsParams) -> anyhow::Result<DevTools> {
///     let observed = app.observed();
///     let link = Link::open(params, observed)?;
///     app.on_cleanup(move || link.close());
///     Ok(DevTools)
/// }
/// ```
#[derive(Clone, Copy)]
pub struct Observed {
    app: Scope,
}

impl Observed {
    pub(crate) fn of(app: Scope) -> Self {
        Self { app }
    }

    /// Starts watching: the first take is the application as it is, each
    /// one after it what moved since. `woken` is called once whenever
    /// something moved after the last take, from wherever it moved - it hands
    /// off and returns. One tool watches at a time.
    pub fn watch(self, woken: impl Fn() + Send + Sync + 'static) -> anyhow::Result<Watching> {
        Ok(observer::watch(self.app, woken)?)
    }

    /// The actor `id`, read now.
    pub fn actor(self, id: usize) -> Option<ActorSnapshot> {
        self.app.actor(id)
    }

    /// The state of actor `id`, read as far as `open` reaches.
    pub fn inspect_actor(self, id: usize, open: &Open) -> Option<Node> {
        self.app.inspect_actor(id, open)
    }

    /// The state `reducer` in `scope`, previewed.
    pub fn state(self, scope: ScopeId, reducer: &str) -> Option<DescribedState> {
        self.app
            .named_by(scope)?
            .describe_states()
            .into_iter()
            .find(|state| state.type_name == reducer)
    }

    /// The state `reducer` in `scope`, read as far as `open` reaches.
    pub fn inspect_state(self, scope: ScopeId, reducer: &str, open: &Open) -> Option<Node> {
        self.app.named_by(scope)?.inspect_state(reducer, open)
    }

    /// What the backend and the plugins offer for the root `root`, or for
    /// the application itself.
    pub fn panels(self, root: Option<u64>) -> Vec<Panel> {
        match root {
            Some(root) => panels::for_root(root),
            None => panels::for_app(),
        }
    }

    /// Sends the action registered as `action`, decoded from `json`, to the
    /// root `root` or the newest one: what is deepest in it is asked first.
    /// Answers the action's id in the trace.
    pub fn act(self, root: Option<u64>, action: &str, json: &str) -> Result<u64, String> {
        let open: Vec<(RootId, Scope)> = roots::opened()
            .into_iter()
            .filter(|(_, window)| self.holds(*window))
            .collect();
        let (_, window) = match root {
            Some(root) => open.into_iter().find(|(id, _)| id.get() == root),
            None => open.into_iter().last(),
        }
        .ok_or_else(|| format!("no window {root:?} is open"))?;

        let mut scopes: Vec<Scope> = window.subtree().into_iter().filter(Scope::is_awake).collect();
        scopes.reverse();
        remote::act_in(&scopes, action, json)
    }

    /// Publishes the event registered as `event`, decoded from `json`, on the
    /// application's bus. Answers its id in the trace.
    pub fn publish(self, event: &str, json: &str) -> Result<u64, String> {
        remote::publish(self.app, event, json)
    }

    fn holds(self, scope: Scope) -> bool {
        std::iter::successors(Some(scope), Scope::parent).any(|above| above == self.app)
    }
}

fn describe(scope: Scope, events: &mut Vec<Event>) {
    if scope.keeps::<Backend>()
        && let Some(Some(name)) = scope.kept::<Backend, _>(|backend| backend.0)
    {
        events.push(Event::Backend { name });
    }
    if scope.keeps::<Plugins>() {
        let built = scope.kept::<Plugins, _>(|plugins| plugins.ids()).unwrap_or_default();
        events.extend(built.into_iter().map(|name| Event::Plugin { name }));
    }
    roots::describe(scope, events);
}

inventory::submit! { Describer(describe) }
