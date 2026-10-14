use std::any::TypeId;
use std::collections::HashMap;

use guinea_core::scope::Scope;

#[derive(Clone, Copy)]
pub(crate) enum Unit {
    Plugin(&'static str),
    Feature(&'static str),
}

impl Unit {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Unit::Plugin(id) => id,
            Unit::Feature(name) => name,
        }
    }

    fn is_plugin(self, id: &str) -> bool {
        matches!(self, Unit::Plugin(own) if own == id)
    }
}

/// The plugins an application was built with, by id: kept in its scope for
/// as long as it lives.
#[derive(Default)]
pub(crate) struct Plugins(HashMap<&'static str, TypeId>);

impl Plugins {
    pub(crate) fn ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.0.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Notes `id` as installed in `app`.
    pub(crate) fn mark(app: Scope, id: &'static str, concrete: TypeId) {
        app.kept::<Plugins, _>(|plugins| plugins.0.insert(id, concrete));
    }

    fn installed(app: Scope, id: &'static str) -> Option<TypeId> {
        app.kept::<Plugins, _>(|plugins| plugins.0.get(id).copied())
            .flatten()
    }
}

/// What is being installed right now, outermost first.
#[derive(Default)]
pub(crate) struct Registry {
    stack: Vec<Unit>,
}

pub(crate) enum Admission {
    Proceed,
    AlreadyInstalled,
}

impl Registry {
    pub(crate) fn current(&self) -> &'static str {
        self.stack.last().map(|unit| unit.label()).unwrap_or("app root")
    }

    pub(crate) fn admit_plugin(
        &self,
        app: Scope,
        id: &'static str,
        concrete: TypeId,
    ) -> anyhow::Result<Admission> {
        if let Some(installed) = Plugins::installed(app, id) {
            anyhow::ensure!(
                installed == concrete,
                "plugin ID collision: `{id}` is already installed by a different type - \
                 two plugins must not share `Plugin::ID`"
            );
            return Ok(Admission::AlreadyInstalled);
        }

        if let Some(at) = self.stack.iter().position(|unit| unit.is_plugin(id)) {
            let path = self.stack[at..]
                .iter()
                .map(|unit| unit.label())
                .collect::<Vec<_>>()
                .join(" -> ");
            anyhow::bail!("plugin dependency cycle: {path} -> {id}");
        }

        Ok(Admission::Proceed)
    }

    pub(crate) fn enter(&mut self, unit: Unit) {
        self.stack.push(unit);
    }

    pub(crate) fn leave(&mut self) {
        self.stack.pop();
    }
}
