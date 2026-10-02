use std::any::TypeId;
use std::collections::{HashMap, HashSet};

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

    fn is_plugin(self, id: &str) -> bool {
        matches!(self, Unit::Plugin(own) if own == id)
    }
}

#[derive(Default)]
pub(crate) struct Registry {
    plugins: HashMap<&'static str, TypeId>,
    features: HashSet<TypeId>,
    stack: Vec<Unit>,
}

pub(crate) enum Admission {
    Proceed,
    AlreadyInstalled,
}

impl Registry {
    pub(crate) fn plugin_ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.plugins.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    pub(crate) fn current(&self) -> &'static str {
        self.stack.last().map(|unit| unit.label()).unwrap_or("app root")
    }

    pub(crate) fn admit_plugin(
        &self,
        id: &'static str,
        concrete: TypeId,
    ) -> anyhow::Result<Admission> {
        if let Some(&installed) = self.plugins.get(id) {
            anyhow::ensure!(
                installed == concrete,
                "plugin ID collision: `{id}` is already installed by a different type - \
                 two plugins must not share `Plugin::ID`"
            );
            return Ok(Admission::AlreadyInstalled);
        }

        if let Some(at) = self.stack.iter().position(|unit| unit.is_plugin(id)) {
            let path = self.stack[at..]
                .iter()
                .map(|unit| unit.label())
                .collect::<Vec<_>>()
                .join(" -> ");
            anyhow::bail!("plugin dependency cycle: {path} -> {id}");
        }

        Ok(Admission::Proceed)
    }

    pub(crate) fn admit_feature(&self, concrete: TypeId, name: &'static str) -> Admission {
        if self.features.contains(&concrete) {
            tracing::warn!(feature = name, "feature installed twice - ignoring the second");
            return Admission::AlreadyInstalled;
        }
        Admission::Proceed
    }

    pub(crate) fn enter(&mut self, unit: Unit) {
        self.stack.push(unit);
    }

    pub(crate) fn leave(&mut self) {
        self.stack.pop();
    }

    pub(crate) fn mark_plugin(&mut self, id: &'static str, concrete: TypeId) {
        self.plugins.insert(id, concrete);
    }

    pub(crate) fn mark_feature(&mut self, concrete: TypeId) {
        self.features.insert(concrete);
    }
}
