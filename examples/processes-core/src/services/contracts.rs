use guinea::prelude::reducer;

// The one action this feature answers. Who answers it is not written down
// anywhere - see `install`.
#[derive(Debug, Clone)]
pub struct Refresh;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Services {
    pub items: Vec<String>,
}

#[derive(Clone)]
pub enum Listed {
    Items(Vec<String>),
}

#[reducer]
fn services(this: &mut Services, update: Listed) {
    match update {
        Listed::Items(items) => this.items = items,
    }
}
