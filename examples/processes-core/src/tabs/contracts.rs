use guinea::prelude::reducer;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Tabs {
    /// What the layout was reached with - `routes!` derives it as the
    /// intersection of the pages under it, so the tab strip navigates to a
    /// sibling with the context it is already in rather than inventing one.
    pub context: String,
    pub install_count: i32,
    pub kills: i32,
    pub last_killed: Option<String>,
}

#[derive(Clone)]
pub enum Chrome {
    Reached(String),
    Installed(i32),
    Killed(String),
}

#[reducer]
fn tabs(this: &mut Tabs, update: Chrome) {
    match update {
        Chrome::Reached(context) => this.context = context,
        Chrome::Installed(n) => this.install_count = n,
        Chrome::Killed(name) => {
            this.kills += 1;
            this.last_killed = Some(name);
        }
    }
}
