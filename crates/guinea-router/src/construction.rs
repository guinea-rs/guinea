//! A route tree, checked as a whole before anything in it mounts.
//!
//! Reach is resolved where a read is made, so what the compiler can no longer
//! refuse is refused here instead - from what each segment lists in
//! `Installs` and what each feature lists in `Exports`, before the first
//! navigation installs anything.

use guinea_app::feature::Listed;
use guinea_core::feature::Named;

use crate::observability::short;
use crate::router::{SegmentEntry, Ui};

/// What the check found: what refuses the tree, and what only deserves a
/// word.
#[derive(Debug, Default, PartialEq)]
pub struct Construction {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Construction {
    fn refuse(&mut self, said: String) {
        if !self.errors.contains(&said) {
            self.errors.push(said);
        }
    }

    fn warn(&mut self, said: String) {
        if !self.warnings.contains(&said) {
            self.warnings.push(said);
        }
    }
}

/// Where something was installed on the chain being walked, and by what.
struct Placed {
    what: Named,
    at: usize,
    by: Named,
}

/// Reads every chain of a tree from what its segments list in `Installs`.
///
/// A chain shares its prefix with its siblings, so a finding is named by the
/// segments that cause it rather than by the route, and said once.
pub fn check<U: Ui>(tree: &[&'static [SegmentEntry<U>]]) -> Construction {
    let mut construction = Construction::default();

    for chain in tree {
        walk(chain, &mut construction);
    }

    construction
}

fn walk<U: Ui>(chain: &[SegmentEntry<U>], construction: &mut Construction) {
    let segment = |at: usize| short((chain[at].type_name)());
    let mut features: Vec<Placed> = Vec::new();
    let mut exports: Vec<Placed> = Vec::new();

    for (at, entry) in chain.iter().enumerate() {
        let mut listed: Vec<Listed> = Vec::new();
        (entry.lists)(&mut listed);

        for item in listed {
            if item.feature {
                match features.iter().find(|placed| placed.what == item.what) {
                    Some(placed) if placed.at == at => construction.refuse(format!(
                        "`{}` is listed twice in what `{}` installs",
                        short(item.what.name),
                        segment(at)
                    )),
                    Some(placed) => construction.refuse(format!(
                        "`{}` is installed by `{}`, and again by `{}` below it",
                        short(item.what.name),
                        segment(placed.at),
                        segment(at)
                    )),
                    None => features.push(Placed {
                        what: item.what,
                        at,
                        by: item.what,
                    }),
                }
            }

            for reducer in &item.exports {
                let earlier = exports
                    .iter()
                    .find(|placed| placed.what == *reducer && placed.by != item.what);
                if let Some(placed) = earlier {
                    construction.warn(if placed.at == at {
                        format!(
                            "`{}` is exported twice by `{}`, through `{}` and `{}`: what reads \
                             it gets whichever claimed it first",
                            short(reducer.name),
                            segment(at),
                            short(placed.by.name),
                            short(item.what.name)
                        )
                    } else {
                        format!(
                            "`{}` is exported by `{}`, and again by `{}` below it: `{}` and \
                             everything below it read the one `{}` installs",
                            short(reducer.name),
                            segment(placed.at),
                            segment(at),
                            segment(at),
                            segment(at)
                        )
                    });
                }

                exports.push(Placed {
                    what: *reducer,
                    at,
                    by: item.what,
                });
            }
        }
    }
}
