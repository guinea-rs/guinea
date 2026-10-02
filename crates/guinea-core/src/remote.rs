//! Actions and events a tool can send from outside, as JSON.
//!
//! Always compiled, and empty until something is registered. A type opts in
//! with `#[derive(guinea::Remote)]` and says which it is -
//! `#[remote(action)]`, `#[remote(event)]` or both - and the derive registers
//! it here. Nothing is reachable by default: what an agent may do to a
//! running application is what its author listed.
//!
//! A type is sent by its short name, `Sort`, or by its path,
//! `app::processes::Sort`. Two pages may each have a `Sort`: the short name
//! means whichever one the open page answers, and only when two would answer
//! in one place does it take the path to say which.
//!
//! An action goes to whichever scope answers it, the way a page's
//! [`Dispatch`](crate::feature::Dispatch) would send it; an event goes out on
//! the global bus as if it had arrived on the UI thread. Either is one
//! [`Point::Action`](crate::trace::Point::Action), and its id is handed back,
//! so what it set off can be followed in the trace.

use serde::de::DeserializeOwned;

use crate::actor::event_bus::{Event, GlobalEventBus};
use crate::actor::short_type_name;
use crate::scope::Scope;
use crate::trace::{self, Point};

/// Hands the decoded action to a scope if the scope answers it: `None` when
/// it does not, the action's id when it did.
pub type Emit = fn(Scope, &str) -> Option<Result<u64, String>>;

/// An action a tool may send, registered by the derive.
pub struct RemoteAction {
    /// The type's name, without its path.
    pub name: &'static str,
    /// The type's module path and name.
    pub path: &'static str,
    /// Whether a scope answers it.
    pub answered_by: fn(Scope) -> bool,
    pub emit: Emit,
}

/// An event a tool may publish, registered by the derive.
pub struct RemoteEvent {
    pub name: &'static str,
    pub path: &'static str,
    pub publish: fn(&str) -> Result<u64, String>,
}

inventory::collect!(RemoteAction);
inventory::collect!(RemoteEvent);

/// Sends the action `named` - a short name or a path - decoded from `json`,
/// to the first of `scopes` that answers one by that name, innermost first:
/// `scopes` run from the outermost layout to the page. Answers the action's
/// id in the trace.
pub fn act_in(scopes: &[Scope], named: &str, json: &str) -> Result<u64, String> {
    let candidates: Vec<&RemoteAction> = inventory::iter::<RemoteAction>()
        .filter(|action| action.name == named || action.path == named)
        .collect();

    if candidates.is_empty() {
        return Err(format!(
            "no action is registered as {named:?} - these are: {:?}",
            actions()
        ));
    }

    for &scope in scopes.iter().rev() {
        let answering: Vec<&RemoteAction> = candidates
            .iter()
            .copied()
            .filter(|action| (action.answered_by)(scope))
            .collect();

        match answering.as_slice() {
            [] => continue,
            [one] => {
                return (one.emit)(scope, json)
                    .unwrap_or_else(|| Err(format!("{} stopped answering", one.path)));
            }
            several => {
                let paths: Vec<&str> = several.iter().map(|action| action.path).collect();
                return Err(format!(
                    "{} actions called {named:?} are answered here - send one by its path: {paths:?}",
                    several.len()
                ));
            }
        }
    }

    Err(format!("nothing on the open page answers {named}"))
}

/// Publishes the event `named` - a short name or a path - decoded from
/// `json`, on the global bus. On the UI thread, where the bus lives.
pub fn publish(named: &str, json: &str) -> Result<u64, String> {
    let candidates: Vec<&RemoteEvent> = inventory::iter::<RemoteEvent>()
        .filter(|event| event.name == named || event.path == named)
        .collect();

    match candidates.as_slice() {
        [] => Err(format!(
            "no event is registered as {named:?} - these are: {:?}",
            events()
        )),
        [one] => (one.publish)(json),
        several => {
            let paths: Vec<&str> = several.iter().map(|event| event.path).collect();
            Err(format!(
                "{} events are called {named:?} - send one by its path: {paths:?}",
                several.len()
            ))
        }
    }
}

/// Every action a tool may send, by name, each once.
pub fn actions() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = inventory::iter::<RemoteAction>()
        .map(|action| action.name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Every event a tool may publish, by name, each once.
pub fn events() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = inventory::iter::<RemoteEvent>()
        .map(|event| event.name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// What the derive registers for an action, to ask a scope about it.
pub fn answered_by<M: 'static>(scope: Scope) -> bool {
    scope.first_answerer::<M>().is_some()
}

/// What the derive registers for an action.
pub fn emit_json<M: DeserializeOwned + 'static>(
    scope: Scope,
    json: &str,
) -> Option<Result<u64, String>> {
    let answer = scope.first_answerer::<M>()?;

    let action: M = match serde_json::from_str(json) {
        Ok(action) => action,
        Err(error) => return Some(Err(format!("{}: {error}", short_type_name::<M>()))),
    };

    let entered = trace::enter(|| Point::Action {
        message: short_type_name::<M>(),
    });
    let cause = entered.id().get();
    answer(action);

    Some(Ok(cause))
}

/// What the derive registers for an event. On the UI thread, where the global
/// bus lives.
pub fn publish_json<M: Event + DeserializeOwned>(json: &str) -> Result<u64, String> {
    let event: M = serde_json::from_str(json)
        .map_err(|error| format!("{}: {error}", short_type_name::<M>()))?;

    let entered = trace::enter(|| Point::Action {
        message: short_type_name::<M>(),
    });
    let cause = entered.id().get();
    GlobalEventBus::bus().publish(event);

    Ok(cause)
}
