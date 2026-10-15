use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::actor::shape::Declared;
use crate::observability::observer::{self, Event};

use super::Scope;

/// A subscription a feature made, as devtools see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Listener {
    pub event: &'static str,
    /// The actor that listens, or `None` for a feature's own callback.
    pub actor: Option<&'static str>,
    pub feature: Option<&'static str>,
}

/// A feature installed in a scope, as devtools see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Installed {
    pub name: &'static str,
    /// Where `impl Feature` was written, when `#[installs]` wrote it.
    pub declared: Option<Declared>,
}

/// One installed feature's corner of a scope, or the scope's own outside any
/// feature: what it claims, answers, makes and hears is its own.
///
/// Named, not kept, like the [`Scope`] it is in. Whoever registers something
/// says which section it is for - a feature's context, an actor's home - so
/// what is made after the feature installed is still the feature's.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Section {
    scope: Scope,
    index: usize,
}

/// What the features installed in a scope brought: which they are, what each
/// answers, the reducers they claimed and the ones they let below.
pub(super) struct Features {
    /// Features (identified by their `install` function's own type) that
    /// were explicitly installed *in this scope*. Apart from the claims
    /// because a feature can spawn several actors/reducers as one unit -
    /// this tracks the unit itself, so a descendant scope can find out "has
    /// an ancestor already taken ownership of this feature" without knowing
    /// which reducer types it happens to use internally.
    installed: RefCell<HashSet<TypeId>>,
    /// One per section opened, in the order they were. Section 0 is the
    /// scope's own.
    sections: RefCell<Vec<SectionData>>,
    /// The reducers claimed here.
    claims: RefCell<HashMap<TypeId, Claimed>>,
    /// The reducers this scope lets segments below it read. See
    /// [`Scope::note_export`].
    ///
    /// Flat, unlike the answers: visibility is not per-instance, and two
    /// instances of one feature export two different reducer types anyway.
    exports: RefCell<HashSet<TypeId>>,
    /// What this scope's features subscribed to.
    listeners: RefCell<Vec<Listener>>,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            installed: RefCell::default(),
            sections: RefCell::new(vec![SectionData::default()]),
            claims: RefCell::default(),
            exports: RefCell::default(),
            listeners: RefCell::default(),
        }
    }
}

#[derive(Default)]
struct SectionData {
    /// The feature it belongs to; `None` for the scope's own, or a plugin's.
    name: Option<&'static str>,
    /// Where the feature was written.
    declared: Option<Declared>,
    /// What it answers.
    ///
    /// Per section, not one map for the scope: an action type is the same for
    /// every instance of a feature, so `ListFeature<Recent>` and
    /// `ListFeature<Archived>` both answer `Refresh`, and a flat map would let
    /// whichever installed last answer for both.
    answers: HashMap<TypeId, Rc<dyn Any>>,
    /// Its feature's install failed: it is not listed as installed.
    withdrawn: bool,
}

/// A reducer claimed in a scope.
#[derive(Clone, Copy)]
pub(super) struct Claimed {
    /// The section that claimed it - what turns "the state I was reading"
    /// into "the instance that owns it".
    pub(super) section: usize,
    /// Where it was claimed.
    pub(super) declared: Option<Declared>,
}

impl Section {
    /// The scope it is a section of.
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The feature it belongs to; `None` for the scope's own, or a plugin's.
    pub fn feature(&self) -> Option<&'static str> {
        self.scope.data()?.features.section_name(self.index)
    }

    /// Where its feature was written, when that is known.
    pub fn declared(&self) -> Option<Declared> {
        let data = self.scope.data()?;
        let sections = data.features.sections.borrow();
        sections.get(self.index)?.declared
    }

    /// Claims `R` here, as written at `declared`. The first claim in the
    /// scope wins.
    pub fn claim<R: 'static>(&self, declared: Option<Declared>) {
        let data = self.scope.alive("claiming a reducer");

        data.features
            .claims
            .borrow_mut()
            .entry(TypeId::of::<R>())
            .or_insert(Claimed {
                section: self.index,
                declared,
            });
    }

    /// Says that this section answers `M`, and how.
    ///
    /// Keyed by the action, not by whoever answers it - which is what keeps
    /// the answerer out of every signature the UI touches. `actor!` calls this
    /// for each handler it lists; a domain that runs on tasks, or a channel,
    /// or a plain closure over a `RefCell`, calls it itself.
    pub fn answers<M: 'static>(&self, answer: impl Fn(M) + 'static) {
        let data = self.scope.alive("answering an action");
        let answer: Rc<dyn Fn(M)> = Rc::new(answer);

        data.features.sections.borrow_mut()[self.index]
            .answers
            .insert(TypeId::of::<M>(), Rc::new(answer) as Rc<dyn Any>);
    }

    /// Takes back what a feature whose install failed said here: the section
    /// is no longer listed, answers nothing and claims nothing. What it made -
    /// state, actors, timers - stays until the scope goes.
    pub fn withdraw(&self) {
        let Some(data) = self.scope.data() else {
            return;
        };

        let withdrawn = data
            .features
            .sections
            .borrow_mut()
            .get_mut(self.index)
            .and_then(|section| {
                let named = section.name.filter(|_| !section.withdrawn);
                section.withdrawn = true;
                section.answers.clear();
                named
            });
        if let Some(name) = withdrawn {
            observer::tell(self.scope, || Event::Withdrawn {
                scope: self.scope.id(),
                name,
            });
        }

        data.features
            .claims
            .borrow_mut()
            .retain(|_, claim| claim.section != self.index);
    }

    /// What answers `M` here, if anything does.
    pub fn answerer<M: 'static>(&self) -> Option<Rc<dyn Fn(M)>> {
        self.scope.data()?.features.answerer::<M>(self.index)
    }

    /// Notes that this section listens to `event` - through `actor` when an
    /// actor does the listening.
    pub fn note_listener(&self, event: &'static str, actor: Option<&'static str>) {
        let data = self.scope.alive("noting a listener");
        let feature = data.features.section_name(self.index);
        let listener = Listener {
            event,
            actor,
            feature,
        };
        data.features.listeners.borrow_mut().push(listener.clone());
        drop(data);

        observer::tell(self.scope, || Event::Listened {
            scope: self.scope.id(),
            listener,
        });
    }
}

impl Scope {
    /// This scope's own section, outside any feature.
    pub fn section(&self) -> Section {
        Section {
            scope: *self,
            index: 0,
        }
    }

    /// Opens a section for what is about to install here - the feature
    /// `name`, or something that is not a feature, such as a plugin.
    pub fn open_section(&self, name: Option<&'static str>, declared: Option<Declared>) -> Section {
        let data = self.alive("installing a feature");
        let mut sections = data.features.sections.borrow_mut();
        sections.push(SectionData {
            name,
            declared,
            answers: HashMap::new(),
            withdrawn: false,
        });
        let index = sections.len() - 1;
        drop(sections);

        if let Some(name) = name {
            observer::tell(*self, || Event::Installed {
                scope: self.id(),
                name,
                declared,
            });
        }
        Section {
            scope: *self,
            index,
        }
    }

    /// Marks feature `F` (its `install` function, used purely as a type
    /// identity) as owned by this scope. `F` must not already be marked
    /// here - two different call sites both claiming ownership of the same
    /// feature in the same scope is a setup bug, not something to merge
    /// silently.
    pub fn mark_feature_installed<F: 'static>(&self) {
        let data = self.alive("installing a feature");
        let newly_inserted = data.features.installed.borrow_mut().insert(TypeId::of::<F>());
        assert!(
            newly_inserted,
            "feature already installed in this scope - install() called twice for the same feature"
        );
    }

    /// Whether anything in this scope has claimed `R`.
    ///
    /// What tells an export that was earned from one that was only declared.
    pub fn claims<R: 'static>(&self) -> bool {
        self.data()
            .is_some_and(|data| data.features.claimed(TypeId::of::<R>()).is_some())
    }

    /// The section that claimed `R` - the instance whose dispatcher a reader
    /// of `R` should be handed - or this scope's own when none did.
    pub fn section_of<R: 'static>(&self) -> Section {
        let index = self
            .data()
            .and_then(|data| data.features.claimed(TypeId::of::<R>()))
            .map_or(0, |claim| claim.section);

        Section {
            scope: *self,
            index,
        }
    }

    /// Every feature installed here, in the order they were.
    pub fn features(&self) -> Vec<Installed> {
        let Some(data) = self.data() else {
            return Vec::new();
        };

        data.features
            .sections
            .borrow()
            .iter()
            .filter(|section| !section.withdrawn)
            .filter_map(|section| {
                Some(Installed {
                    name: section.name?,
                    declared: section.declared,
                })
            })
            .collect()
    }

    pub fn listeners(&self) -> Vec<Listener> {
        self.data()
            .map(|data| data.features.listeners.borrow().clone())
            .unwrap_or_default()
    }

    /// Whether feature `F` was marked installed in *this exact* scope.
    pub fn has_feature<F: 'static>(&self) -> bool {
        self.data()
            .is_some_and(|data| data.features.installed.borrow().contains(&TypeId::of::<F>()))
    }

    /// Marks `R` as readable from segments below this one.
    ///
    /// Called by `cx.install::<F>()` for everything in `F::Exports`, and by
    /// an application's `export::<R>()`. A reducer a feature claimed but did not export stays
    /// visible to the feature itself and invisible from below - which is the
    /// whole difference between a feature and a folder.
    pub fn note_export<R: 'static>(&self) {
        let data = self.alive("exporting a reducer");
        data.features.exports.borrow_mut().insert(TypeId::of::<R>());
    }

    /// Whether `R` is readable from below this scope.
    pub fn exports<R: 'static>(&self) -> bool {
        self.data()
            .is_some_and(|data| data.features.exports.borrow().contains(&TypeId::of::<R>()))
    }

    /// What answers `M` anywhere in this scope - the first section that does,
    /// in the order they were opened. For a sender that knows the action and
    /// not which state it was reading.
    pub fn first_answerer<M: 'static>(&self) -> Option<Rc<dyn Fn(M)>> {
        let data = self.data()?;
        let sections = data.features.sections.borrow().len();
        (0..sections).find_map(|section| data.features.answerer::<M>(section))
    }
}

impl Features {
    pub(super) fn section_name(&self, section: usize) -> Option<&'static str> {
        self.sections.borrow().get(section)?.name
    }

    pub(super) fn claimed(&self, reducer: TypeId) -> Option<Claimed> {
        self.claims.borrow().get(&reducer).copied()
    }

    fn answerer<M: 'static>(&self, section: usize) -> Option<Rc<dyn Fn(M)>> {
        let sections = self.sections.borrow();
        let answer = sections.get(section)?.answers.get(&TypeId::of::<M>())?.clone();
        answer.downcast::<Rc<dyn Fn(M)>>().ok().map(|a| (*a).clone())
    }
}
