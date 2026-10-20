use std::any::{TypeId, type_name};

use guinea_core::actor::shape::Declared;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Service,
    Feed,
    State,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub id: TypeId,
    pub name: &'static str,
    pub channel: Channel,
    pub optional: bool,
}

impl Port {
    pub fn of<T: 'static>(channel: Channel) -> Self {
        Self {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
            channel,
            optional: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    fn matches(&self, other: &Self) -> bool {
        self.id == other.id && self.channel == other.channel
    }
}

#[derive(Clone, Debug)]
pub struct Declaration {
    pub id: TypeId,
    pub name: &'static str,
    pub kind: &'static str,
    pub source: Declared,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
}

impl Declaration {
    pub fn of<T: 'static>(
        kind: &'static str,
        source: Declared,
        inputs: Vec<Port>,
        outputs: Vec<Port>,
    ) -> Self {
        Self {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
            kind,
            source,
            inputs,
            outputs,
        }
    }
}

pub struct Registration(pub fn() -> Declaration);

inventory::collect!(Registration);

pub struct TemplateRegistration {
    pub target: &'static str,
    pub signature: &'static str,
    pub source: Declared,
}

inventory::collect!(TemplateRegistration);

pub struct TemplateManifest {
    pub name: &'static str,
    pub exports: &'static [&'static str],
    pub feeds: &'static [&'static str],
    pub provides: &'static [&'static str],
}

inventory::collect!(TemplateManifest);

pub fn registered() -> Vec<Declaration> {
    let mut declarations: Vec<_> = inventory::iter::<Registration>
        .into_iter()
        .map(|entry| (entry.0)())
        .collect();
    declarations.sort_by_key(|declaration| {
        (
            declaration.name,
            declaration.source.file,
            declaration.source.line,
        )
    });
    declarations
}

pub fn errors(declarations: &[Declaration]) -> Option<String> {
    errors_with_templates(declarations, &[])
}

fn errors_with_templates(declarations: &[Declaration], deferred: &[Channel]) -> Option<String> {
    use crate::diagnostic::Diagnostic;
    let mut failures = Vec::new();
    for declaration in declarations {
        let mut problems = Vec::new();
        for (index, output) in declaration.outputs.iter().enumerate() {
            if declaration.outputs[..index]
                .iter()
                .any(|other| output.matches(other))
            {
                problems.push(Diagnostic::new(format!(
                    "duplicate {:?} output `{}`",
                    output.channel, output.name
                )));
            }
            if output.channel == Channel::Service {
                let providers: Vec<_> = declarations
                    .iter()
                    .filter(|provider| {
                        provider.id != declaration.id
                            && provider.outputs.iter().any(|other| output.matches(other))
                    })
                    .collect();
                if !providers.is_empty() {
                    problems.push(
                        Diagnostic::new(format!("conflicting providers of `{}`", output.name))
                            .with_related(providers.into_iter().map(provider_diagnostic).collect()),
                    );
                }
            }
        }
        for (index, input) in declaration.inputs.iter().enumerate() {
            if declaration.inputs[..index]
                .iter()
                .any(|other| input.matches(other) && input.optional != other.optional)
            {
                problems.push(Diagnostic::new(format!(
                    "input `{}` is both required and optional",
                    input.name
                )));
            }
            let providers: Vec<_> = declarations
                .iter()
                .filter(|provider| provider.outputs.iter().any(|output| input.matches(output)))
                .collect();
            match providers.as_slice() {
                [] if !input.optional && !deferred.contains(&input.channel) => problems.push(
                    Diagnostic::new(format!("requires {:?} `{}`", input.channel, input.name))
                        .with_related(vec![Diagnostic::new("no declared provider")]),
                ),
                [_, _, ..] if input.channel == Channel::Service => {
                    problems.push(
                        Diagnostic::new(format!("ambiguous {:?} `{}`", input.channel, input.name))
                            .with_related(providers.into_iter().map(provider_diagnostic).collect()),
                    );
                }
                _ => {}
            }
        }
        if !problems.is_empty() {
            failures.push(provider_diagnostic(declaration).with_related(problems));
        }
    }
    let mut graph = petgraph::graph::DiGraph::<&Declaration, &Port>::new();
    let nodes: Vec<_> = declarations
        .iter()
        .map(|declaration| graph.add_node(declaration))
        .collect();
    for (index, declaration) in declarations.iter().enumerate() {
        for input in declaration
            .inputs
            .iter()
            .filter(|input| input.channel == Channel::Service)
        {
            for (provider_index, provider) in declarations.iter().enumerate() {
                if provider.outputs.iter().any(|output| input.matches(output)) {
                    graph.add_edge(nodes[index], nodes[provider_index], input);
                }
            }
        }
    }
    for mut component in petgraph::algo::kosaraju_scc(&graph) {
        if component.len() == 1 && graph.find_edge(component[0], component[0]).is_none() {
            continue;
        }
        component.sort_by_key(|node| graph[*node].name);
        use petgraph::visit::EdgeRef;
        let mut participants = Vec::new();
        for node in &component {
            let mut dependencies: Vec<_> = graph
                .edges(*node)
                .filter(|edge| component.contains(&edge.target()))
                .map(|edge| {
                    format!(
                        "requires `{}` from {}",
                        edge.weight().name,
                        graph[edge.target()].name
                    )
                })
                .collect();
            dependencies.sort();
            participants.push(
                provider_diagnostic(graph[*node])
                    .with_related(dependencies.into_iter().map(Diagnostic::new).collect()),
            );
        }
        failures.push(Diagnostic::new("dependency cycle").with_related(participants));
    }
    (!failures.is_empty()).then(|| {
        Diagnostic::new("Invalid guinea declarations")
            .with_related(failures)
            .render()
    })
}

pub(crate) fn provider_diagnostic(declaration: &Declaration) -> crate::diagnostic::Diagnostic {
    crate::diagnostic::Diagnostic::new(format!(
        "{} {} at {}:{}:{}",
        declaration.kind,
        declaration.name,
        declaration.source.file.replace('\\', "/"),
        declaration.source.line,
        declaration.source.column,
    ))
}

pub fn validate() {
    let mut deferred = Vec::new();
    for template in inventory::iter::<TemplateManifest> {
        if !template.provides.is_empty() {
            deferred.push(Channel::Service);
        }
        if !template.feeds.is_empty() {
            deferred.push(Channel::Feed);
        }
        if !template.exports.is_empty() {
            deferred.push(Channel::State);
        }
    }
    if let Some(report) = errors_with_templates(&registered(), &deferred) {
        panic!("{report}");
    }
}

#[derive(Default)]
struct ConcreteDeclarations(Vec<Declaration>);

pub(crate) fn note(scope: guinea_core::scope::Scope, declaration: &Declaration) {
    let root = scope.ancestors().last().copied().unwrap_or(scope);
    if !root.keeps::<ConcreteDeclarations>() {
        root.keep(ConcreteDeclarations::default());
    }
    root.kept::<ConcreteDeclarations, _>(|declared| {
        if !declared
            .0
            .iter()
            .any(|previous| previous.id == declaration.id)
        {
            declared.0.push(declaration.clone());
        }
    });
}

pub(crate) fn in_scope(scope: guinea_core::scope::Scope) -> Vec<Declaration> {
    let mut declarations = registered();
    let root = scope.ancestors().last().copied().unwrap_or(scope);
    if let Some(recorded) = root.try_kept::<ConcreteDeclarations, _>(|declared| declared.0.clone())
    {
        for declaration in recorded {
            if !declarations
                .iter()
                .any(|previous| previous.id == declaration.id)
            {
                declarations.push(declaration);
            }
        }
    }
    declarations.sort_by_key(|declaration| {
        (
            declaration.name,
            declaration.source.file,
            declaration.source.line,
        )
    });
    declarations
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Application;
    struct Settings;
    struct Consumer;
    struct Store;
    struct Selection;

    fn declaration<T: 'static>(inputs: Vec<Port>, outputs: Vec<Port>) -> Declaration {
        Declaration::of::<T>(
            "feature",
            Declared {
                file: "fixture.rs",
                line: 42,
                column: 1,
                crate_dir: "fixture",
            },
            inputs,
            outputs,
        )
    }

    #[test]
    fn a_complete_graph_is_valid_without_executing_any_installers() {
        let provider = declaration::<Settings>(vec![], vec![Port::of::<Store>(Channel::Service)]);
        let consumer = declaration::<Consumer>(vec![Port::of::<Store>(Channel::Service)], vec![]);
        assert!(errors(&[provider, consumer]).is_none());
    }

    #[test]
    fn missing_inputs_form_a_tree_with_source_locations() {
        let consumer = declaration::<Consumer>(
            vec![
                Port::of::<Store>(Channel::Service),
                Port::of::<Selection>(Channel::Feed),
            ],
            vec![],
        );
        let report = errors(&[consumer]).unwrap();
        insta::assert_snapshot!("missing_inputs", report);
    }

    #[test]
    fn conflicting_providers_are_rejected_even_without_consumers() {
        let first = declaration::<Settings>(vec![], vec![Port::of::<Store>(Channel::Service)]);
        let second = declaration::<Application>(vec![], vec![Port::of::<Store>(Channel::Service)]);
        let report = errors(&[first, second]).unwrap();
        insta::assert_snapshot!("conflicting_providers", report);
    }

    #[test]
    fn an_optional_input_without_a_provider_is_valid() {
        let consumer =
            declaration::<Consumer>(vec![Port::of::<Store>(Channel::Service).optional()], vec![]);
        assert!(errors(&[consumer]).is_none());
    }

    #[test]
    fn optional_and_required_inputs_cannot_disagree() {
        let consumer = declaration::<Consumer>(
            vec![
                Port::of::<Store>(Channel::Service),
                Port::of::<Store>(Channel::Service).optional(),
            ],
            vec![],
        );
        insta::assert_snapshot!("contradicting_inputs", errors(&[consumer]).unwrap());
    }

    #[test]
    fn service_and_feed_outputs_do_not_satisfy_each_other() {
        let provider =
            declaration::<Settings>(vec![], vec![Port::of::<Selection>(Channel::Service)]);
        let consumer = declaration::<Consumer>(vec![Port::of::<Selection>(Channel::Feed)], vec![]);
        insta::assert_snapshot!("different_channels", errors(&[provider, consumer]).unwrap());
    }

    #[test]
    fn dependency_cycles_include_the_complete_chain() {
        let first = declaration::<Settings>(
            vec![Port::of::<Selection>(Channel::Service)],
            vec![Port::of::<Store>(Channel::Service)],
        );
        let second = declaration::<Application>(
            vec![Port::of::<Store>(Channel::Service).optional()],
            vec![Port::of::<Selection>(Channel::Service)],
        );
        let report = errors(&[first, second]).unwrap();
        insta::assert_snapshot!("dependency_cycles", report);
    }

    #[test]
    fn output_duplicates_are_rejected() {
        let provider = declaration::<Settings>(
            vec![],
            vec![
                Port::of::<Store>(Channel::Service),
                Port::of::<Store>(Channel::Service),
            ],
        );
        insta::assert_snapshot!("duplicate_outputs", errors(&[provider]).unwrap());
    }
}
