use guinea_core::__private::observer;
use guinea_core::observability::Event;
use guinea_core::scope::Scope;


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
        observer::tell(app, || Event::Plugin { name: id });
    }
}

/// What draws the application, as the backend that started it named itself.
#[derive(Default)]
pub(crate) struct Backend(pub(crate) Option<&'static str>);
