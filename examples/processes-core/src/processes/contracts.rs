//! The feature's state, and this file is the point of the whole exercise:
//! a struct, an enum, and the function that changes the one by the other.
//!
//! `#[reducer]` writes the impl from the function's signature and names
//! nothing itself - the central type of the feature, the one every other file
//! names, is the one written here.

use guinea::prelude::reducer;
use serde::Deserialize;

// What the actor answers to: kill the process with this pid, and list them
// again. Which actor that is, is settled where the two are already listed
// together - in `actor!` - and not here: this file knows state, and an actor
// named in it would be exactly the leak the layering exists to prevent.
//
// Both can be sent from devtools, or by an agent through them: `Remote` lists
// them, by name, among what a tool may do to the running application.
#[derive(Debug, Clone, Deserialize, guinea::Remote)]
#[remote(action)]
pub struct Kill(pub u32);

#[derive(Debug, Clone, Deserialize, guinea::Remote)]
#[remote(action)]
pub struct Refresh;

/// The state, which is the reducer.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Processes {
    pub items: Vec<String>,
}

/// What changes it. Only the actor produces these.
#[derive(Clone)]
pub enum Listed {
    Items(Vec<String>),
}

#[reducer]
fn processes(this: &mut Processes, update: Listed) {
    match update {
        Listed::Items(items) => this.items = items,
    }
}
