//! A named unit with its own lifetime, and one expression for the whole of it.
//!
//! The actor is created by the very call that claims the reducer, so the pair
//! is known by construction: there is nothing to wire and nothing to leave
//! unwired. `context` is what the route captured, handed over typed - the
//! feature never sees an address, so there is no segment index to get wrong.

use guinea::prelude::*;

use super::actor::ProcessActor;
use super::contracts::{self, Refresh};

feature! {
    /// One reducer, and pages below may read it. Anything else this feature
    /// claimed would stay its own.
    pub ProcessesFeature {
        exports { contracts::Processes }
    }
}

#[installs]
fn processes(cx: &FeatureInitContext, context: &str) -> anyhow::Result<ProcessesFeature> {
    let (listing, _) = cx
        .state::<contracts::Processes>()
        .driven_by(|push| ProcessActor::new(context.to_string(), push));

    listing.emit(Refresh);
    Ok(ProcessesFeature(listing))
}
