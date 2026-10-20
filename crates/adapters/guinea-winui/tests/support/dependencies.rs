guinea_macros::plugin! {
    pub Fixture<Dependency> {
        provides { Dependency }
    }
}

#[guinea_macros::builds]
fn fixture<Dependency: Clone + 'static>(
    _app: &mut guinea_app::app::PluginBuilder,
    dependency: Dependency,
) -> anyhow::Result<Fixture<Dependency>> {
    Ok(Fixture(dependency))
}
