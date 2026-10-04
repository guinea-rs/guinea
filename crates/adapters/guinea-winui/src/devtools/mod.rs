//! A window's component tree, for devtools.
//!
//! The reactor keeps its mounted tree to itself, so this is built from what
//! guinea does see: every segment's last `View`, read back from `Debug`, with
//! each segment's own output nested where its parent left the outlet.

mod debug_text;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use guinea_core::observability::panels::{Panel, PanelNode};
use guinea_core::scope::Scope;
use guinea_router::router::{Router, SegmentProps};
use windows_reactor::View;

use crate::winui::WinUi;
use debug_text::Value;

struct Record {
    renders: u64,
    view: Option<String>,
}

thread_local! {
    static RECORDS: RefCell<HashMap<Scope, Record>> = RefCell::new(HashMap::new());
}

/// Notes that the segment `props` points at produced `view`, while devtools
/// watch; otherwise does nothing, not even count.
pub(crate) fn record(props: &SegmentProps<WinUi>, view: &View) {
    if !guinea_core::observability::is_observed() {
        return;
    }

    let Some(&scope) = props.scopes.get(props.cursor) else {
        return;
    };
    let text = format!("{view:?}");

    RECORDS.with(|records| {
        let mut records = records.borrow_mut();
        records.retain(|scope, _| scope.is_alive());
        let record = records.entry(scope).or_insert_with(|| Record {
            renders: 0,
            view: None,
        });
        record.renders += 1;
        record.view = Some(text);
    });
}

/// Offers the components panel for `router`'s root while the guard lives.
pub(crate) fn offer(router: &Rc<Router<WinUi>>) -> guinea_core::observability::panels::PanelGuard {
    let weak = Rc::downgrade(router);
    guinea_core::observability::panels::contribute(router.root().get(), move || {
        weak.upgrade().map(|router| panel(&router))
    })
}

fn panel(router: &Router<WinUi>) -> Panel {
    let segments: Vec<(&'static str, u64, Option<Value>)> =
        match (router.active_chain(), router.active_scopes()) {
            (Some(chain), Some(scopes)) => RECORDS.with(|records| {
                let records = records.borrow();
                chain
                    .iter()
                    .zip(scopes.iter())
                    .map(|(entry, scope)| {
                        let record = records.get(scope);
                        (
                            short((entry.type_name)()),
                            record.map_or(0, |record| record.renders),
                            record
                                .and_then(|record| record.view.as_deref())
                                .and_then(debug_text::parse),
                        )
                    })
                    .collect()
            }),
            _ => Vec::new(),
        };

    Panel {
        id: "winui.components",
        title: "Components",
        nodes: segment(&segments, 0).into_iter().collect(),
    }
}

fn segment(segments: &[(&'static str, u64, Option<Value>)], at: usize) -> Option<PanelNode> {
    let (name, renders, view) = segments.get(at)?;
    let mut outlet = Some(at + 1);
    let children = match view {
        Some(view) => views(view, &mut |_| {
            outlet
                .take()
                .and_then(|next| segment(segments, next))
                .into_iter()
                .collect()
        }),
        None => Vec::new(),
    };
    Some(PanelNode {
        label: name.to_string(),
        kind: "segment".into(),
        properties: vec![("renders".into(), renders.to_string())],
        children,
    })
}

type Outlet<'a> = dyn FnMut(&str) -> Vec<PanelNode> + 'a;

fn views(value: &Value, outlet: &mut Outlet<'_>) -> Vec<PanelNode> {
    match value.name() {
        Some("View") | Some("Native") => value
            .single()
            .map(|inner| views(inner, outlet))
            .unwrap_or_default(),
        Some("Children") => {
            let mut node = control(value.field("control"));
            if let Some(Value::List(items)) = value.field("children") {
                node.children = items.iter().flat_map(|item| views(item, outlet)).collect();
            }
            vec![node]
        }
        Some("Content") => {
            let mut node = control(value.field("control"));
            if let Some(content) = value.field("content") {
                node.children = views(content, outlet);
            }
            vec![node]
        }
        Some("Fragment") => match value.single() {
            Some(Value::List(items)) => items.iter().flat_map(|item| views(item, outlet)).collect(),
            _ => Vec::new(),
        },
        Some("KeyedView") => {
            let mut nodes = value
                .field("view")
                .map(|view| views(view, outlet))
                .unwrap_or_default();
            if let (Some(key), Some(first)) = (named_key(value.field("key")), nodes.first_mut()) {
                first.properties.insert(0, ("key".into(), key));
            }
            nodes
        }
        Some("Component") => {
            let mut inner = value;
            while let Some(next) = inner.single() {
                inner = next;
            }
            let name = match inner {
                Value::Atom(name) => name.trim_matches('"').to_string(),
                _ => String::new(),
            };
            if name.contains("PageNode<") || name.contains("LayoutNode<") {
                return outlet(&name);
            }
            vec![PanelNode {
                label: short(&name).to_string(),
                kind: "component".into(),
                properties: Vec::new(),
                children: Vec::new(),
            }]
        }
        Some(_) if matches!(value, Value::Tuple { .. }) && value.single().is_some() => {
            vec![control(Some(value))]
        }
        Some(name) => vec![PanelNode {
            label: name.to_string(),
            kind: "view".into(),
            properties: Vec::new(),
            children: Vec::new(),
        }],
        None => Vec::new(),
    }
}

/// `Button(Button { .. })`: the kind, and the properties that were set.
fn control(value: Option<&Value>) -> PanelNode {
    let Some(value) = value else {
        return PanelNode::default();
    };
    let label = value.name().unwrap_or_default().to_string();
    let body = value.single().unwrap_or(value);
    let properties = match body {
        Value::Struct { fields, .. } => fields
            .iter()
            .filter_map(|(name, field)| match field {
                Value::Tuple { name: set, items } if set == "Set" && items.len() == 1 => {
                    Some((name.clone(), items[0].compact()))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let kind = properties
        .iter()
        .find(|(name, _)| matches!(name.as_str(), "text" | "content" | "header" | "title"))
        .map_or_else(|| "native".to_string(), |(_, value)| value.clone());
    PanelNode {
        label,
        kind,
        properties,
        children: Vec::new(),
    }
}

/// A key the application chose; positions are not worth showing.
fn named_key(value: Option<&Value>) -> Option<String> {
    let inner = value?.single()?;
    match inner.name() {
        Some("Position") => None,
        _ => Some(inner.single().map_or_else(|| inner.compact(), Value::compact)),
    }
}

fn short(name: &str) -> &str {
    let generic = name.find('<').unwrap_or(name.len());
    let start = name[..generic].rfind("::").map_or(0, |at| at + 2);
    &name[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(text: &str, outlet: &str) -> Vec<PanelNode> {
        let value = debug_text::parse(text).unwrap();
        views(&value, &mut |_| {
            vec![PanelNode {
                label: outlet.to_string(),
                ..PanelNode::default()
            }]
        })
    }

    #[test]
    fn a_layout_nests_its_page_where_the_outlet_was() {
        let nodes = tree(
            r#"View(Children {
                control: StackPanel(StackPanel { spacing: Set(12.0), orientation: Inherited }),
                children: [
                    KeyedView { key: Key(Position(0)), view: View(Native(TextBlock(TextBlock { text: Set("Tabs") }))) },
                    KeyedView { key: Key(String("page")), view: View(Component(Component("guinea_winui::winui::PageNode<app::Metrics>"))) },
                    KeyedView { key: Key(Position(2)), view: View(Component(Component("app::widgets::Chart"))) },
                ],
            })"#,
            "Metrics",
        );

        assert_eq!(nodes.len(), 1);
        let panel = &nodes[0];
        assert_eq!(panel.label, "StackPanel");
        assert_eq!(panel.properties, [("spacing".to_string(), "12.0".to_string())]);

        let labels: Vec<&str> = panel.children.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["TextBlock", "Metrics", "Chart"]);
        assert_eq!(panel.children[0].kind, "\"Tabs\"");
        assert_eq!(panel.children[2].kind, "component");
    }

    #[test]
    fn a_content_control_holds_its_content_and_a_named_key_is_kept() {
        let nodes = tree(
            r#"View(Fragment([
                KeyedView { key: Key(String("kill")), view: View(Content {
                    control: Button(Button { is_enabled: Inherited }),
                    content: Native(TextBlock(TextBlock { text: Set("Kill") })),
                }) },
            ]))"#,
            "unused",
        );

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].label, "Button");
        assert_eq!(nodes[0].properties, [("key".to_string(), "\"kill\"".to_string())]);
        assert_eq!(nodes[0].children[0].label, "TextBlock");
    }
}
