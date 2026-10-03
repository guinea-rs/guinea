use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::rc::Rc;

use crate::actor::shape::Declared;

use super::{Scope, ScopeData};

/// A subscription a feature made, as devtools see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Listener {
    pub event: &'static str,
    /// The actor that listens, or `None` for a feature's own callback.
    pub actor: Option<&'static str>,
    pub bus: crate::trace::Bus,
    pub feature: Option<&'static str>,
}

/// A feature installed in a scope, as devtools see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Installed {
    pub name: &'static str,
    /// Where `impl Feature` was written, when `#[installs]` wrote it.
    pub declared: Option<Declared>,
}

impl Scope {
    /// Marks feature `F` (its `install` function, used purely as a type
    /// identity) as owned by this scope. `F` must not already be marked
    /// here - two different call sites both claiming ownership of the same
    /// feature in the same scope is a setup bug, not something to merge
    /// silently.
    pub fn mark_feature_installed<F: 'static>(&self) {
        let data = self.installing("installing a feature");
        let newly_inserted = data.installed_features.borrow_mut().insert(TypeId::of::<F>());
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
            .is_some_and(|data| data.owners.borrow().contains_key(&TypeId::of::<R>()))
    }

    pub fn note_reducer_owner<R: 'static>(&self) {
        let data = self.installing("claiming a reducer");
        data.installed_features.borrow_mut().insert(TypeId::of::<R>());
        let section = data.current_section();
        data.owners.borrow_mut().entry(TypeId::of::<R>()).or_insert(section);
    }

    /// Notes where reducer `R` was claimed, the first claim winning.
    pub fn note_reducer_declared<R: 'static>(&self, declared: Declared) {
        let data = self.installing("claiming a reducer");
        data.declarations.borrow_mut().entry(TypeId::of::<R>()).or_insert(declared);
    }

    /// The manifest directory of the feature installing now, for a claim that
    /// only knows the file the compiler gave it.
    pub fn current_crate_dir(&self) -> Option<&'static str> {
        let data = self.data()?;
        let section = data.current_section();
        let declarations = data.section_declarations.borrow();
        declarations.get(section).copied().flatten().map(|declared| declared.crate_dir)
    }

    /// Opens a section for what is about to install - the feature `name`, or
    /// something that is not a feature, such as a plugin. Returns its index.
    pub fn open_section(&self, name: Option<&'static str>, declared: Option<Declared>) -> usize {
        let data = self.installing("installing a feature");
        let mut sections = data.sections.borrow_mut();
        if sections.is_empty() {
            sections.push(HashMap::new());
        }
        sections.push(HashMap::new());
        let index = sections.len() - 1;
        let mut names = data.section_names.borrow_mut();
        names.resize(index + 1, None);
        names[index] = name;

        let mut declarations = data.section_declarations.borrow_mut();
        declarations.resize(index + 1, None);
        declarations[index] = declared;

        data.installing.borrow_mut().push(index);
        index
    }

    /// The feature a section belongs to; `None` for the segment's own.
    pub fn section_name(&self, section: usize) -> Option<&'static str> {
        self.data()?.section_name(section)
    }

    /// Every feature installed here, in the order they were.
    pub fn features(&self) -> Vec<Installed> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let declarations = data.section_declarations.borrow();

        data.section_names
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(section, name)| {
                Some(Installed {
                    name: (*name)?,
                    declared: declarations.get(section).copied().flatten(),
                })
            })
            .collect()
    }

    /// The feature being installed right now, if any.
    pub fn current_feature(&self) -> Option<&'static str> {
        let data = self.data()?;
        data.section_name(data.current_section())
    }

    /// Notes that whatever is installing listens to `event` on `bus` - through
    /// `actor` when an actor does the listening.
    pub fn note_listener(
        &self,
        event: &'static str,
        actor: Option<&'static str>,
        bus: crate::trace::Bus,
    ) {
        let Some(data) = self.data() else { return };
        let feature = data.section_name(data.current_section());
        data.listeners.borrow_mut().push(Listener {
            event,
            actor,
            bus,
            feature,
        });
    }

    pub fn listeners(&self) -> Vec<Listener> {
        self.data()
            .map(|data| data.listeners.borrow().clone())
            .unwrap_or_default()
    }

    pub fn close_section(&self) {
        if let Some(data) = self.data() {
            data.installing.borrow_mut().pop();
        }
    }

    /// The section being installed, or the segment's own when none is.
    pub fn current_section(&self) -> usize {
        self.data().map_or(0, |data| data.current_section())
    }

    /// Which section owns `R` - the instance whose dispatcher a reader of `R`
    /// should be handed.
    pub fn section_of<R: 'static>(&self) -> usize {
        self.data()
            .and_then(|data| data.owners.borrow().get(&TypeId::of::<R>()).copied())
            .unwrap_or(0)
    }

    /// Whether feature `F` was marked installed in *this exact* scope.
    pub fn has_feature<F: 'static>(&self) -> bool {
        self.data()
            .is_some_and(|data| data.installed_features.borrow().contains(&TypeId::of::<F>()))
    }

    /// Marks `R` as readable from segments below this one.
    ///
    /// Called by `cx.install::<F>()` for everything in `F::Exports`, and by
    /// an application's `export::<R>()`. A reducer a feature claimed but did not export stays
    /// visible to the feature itself and invisible from below - which is the
    /// whole difference between a feature and a folder.
    pub fn note_export<R: 'static>(&self) {
        let data = self.installing("exporting a reducer");
        data.exports.borrow_mut().insert(TypeId::of::<R>());
    }

    /// Whether `R` is readable from below this scope.
    pub fn exports<R: 'static>(&self) -> bool {
        self.data()
            .is_some_and(|data| data.exports.borrow().contains(&TypeId::of::<R>()))
    }

    /// Says that this scope answers `M`, and how.
    ///
    /// Keyed by the action, not by whoever answers it - which is what keeps
    /// the answerer out of every signature the UI touches. `actor!` calls this
    /// for each handler it lists; a domain that runs on tasks, or a channel,
    /// or a plain closure over a `RefCell`, calls it itself.
    pub fn answers<M: 'static>(&self, answer: impl Fn(M) + 'static) {
        let data = self.installing("answering an action");
        let answer: Rc<dyn Fn(M)> = Rc::new(answer);
        let section = data.current_section();

        let mut sections = data.sections.borrow_mut();
        while sections.len() <= section {
            sections.push(HashMap::new());
        }
        sections[section].insert(TypeId::of::<M>(), Rc::new(answer) as Rc<dyn Any>);
    }

    /// What answers `M` in one section of this scope, if anything does.
    pub fn answerer<M: 'static>(&self, section: usize) -> Option<Rc<dyn Fn(M)>> {
        self.data()?.answerer::<M>(section)
    }

    /// What answers `M` anywhere in this scope - the first feature that does,
    /// in the order they installed. For a sender that knows the action and
    /// not which state it was reading.
    pub fn first_answerer<M: 'static>(&self) -> Option<Rc<dyn Fn(M)>> {
        let data = self.data()?;
        let sections = data.sections.borrow().len();
        (0..sections).find_map(|section| data.answerer::<M>(section))
    }
}

impl ScopeData {
    fn answerer<M: 'static>(&self, section: usize) -> Option<Rc<dyn Fn(M)>> {
        let sections = self.sections.borrow();
        let answer = sections.get(section)?.get(&TypeId::of::<M>())?.clone();
        answer.downcast::<Rc<dyn Fn(M)>>().ok().map(|a| (*a).clone())
    }
}
