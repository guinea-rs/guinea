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
}

/// The plugins an application was built with, in the order they were, once
/// per copy: kept in its scope for as long as it lives.
#[derive(Default)]
pub(crate) struct Plugins(Vec<&'static str>);

impl Plugins {
    pub(crate) fn ids(&self) -> Vec<&'static str> {
        self.0.clone()
    }

    /// Notes a copy of `id` as installed in `app`.
    pub(crate) fn mark(app: Scope, id: &'static str) {
        app.kept::<Plugins, _>(|plugins| plugins.0.push(id));
    }
}

/// What is being installed right now, outermost first.
#[derive(Default)]
pub(crate) struct Registry {
    stack: Vec<Unit>,
}

impl Registry {
    pub(crate) fn current(&self) -> &'static str {
        self.stack.last().map(|unit| unit.label()).unwrap_or("app root")
    }

    pub(crate) fn enter(&mut self, unit: Unit) {
        self.stack.push(unit);
    }

    pub(crate) fn leave(&mut self) {
        self.stack.pop();
    }
}
