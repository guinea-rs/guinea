use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;

use super::pick::{Probe, ViaDebug as _, ViaInspect as _, ViaNothing as _};
use super::*;

struct Process {
    pid: u32,
    name: String,
}

impl Inspect for Process {
    fn inspect(&self, at: Inspector<'_>) {
        at.record("Process")
            .field("pid", &self.pid)
            .field("name", &self.name)
            .finish();
    }
}

fn process(pid: u32) -> Process {
    Process {
        pid,
        name: format!("p{pid}"),
    }
}

/// Counts how many times it was asked to show itself.
struct Counted<'c> {
    n: usize,
    asked: &'c Cell<usize>,
}

impl Inspect for Counted<'_> {
    fn inspect(&self, at: Inspector<'_>) {
        self.asked.set(self.asked.get() + 1);
        at.value(&self.n);
    }
}

fn open() -> Open {
    Open::default()
}

fn opened_at(steps: &[Step]) -> Open {
    steps.iter().rev().fold(open(), |inner, step| Open {
        window: None,
        children: vec![(step.clone(), inner)],
    })
}

fn children(node: &Node) -> &[(Step, Node)] {
    match &node.opened {
        Some(Opened::Children(children)) => children,
        other => panic!("not opened into children: {other:?}"),
    }
}

fn field(name: &str) -> Step {
    Step::Field(name.to_string())
}

#[test]
fn a_record_previews_on_one_line() {
    let node = Node::of(&process(4), None);

    assert_eq!(node.form, Form::Record);
    assert_eq!(node.preview, r#"Process { pid: 4, name: "p4" }"#);
    assert_eq!(node.opened, None, "nothing was asked open");
}

#[test]
fn an_open_record_gives_its_fields() {
    let node = Node::of(&process(4), Some(&open()));

    let fields = children(&node);
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0, field("pid"));
    assert_eq!(fields[0].1.preview, "4");
    assert_eq!(fields[0].1.form, Form::Value);
    assert_eq!(fields[1].1.preview, r#""p4""#);
}

#[test]
fn a_tuple_previews_without_names() {
    let node = Node::of(&(1, "two"), Some(&open()));

    assert_eq!(node.preview, r#"(1, "two")"#);
    assert_eq!(children(&node)[1].0, field("1"));
    assert_eq!(Node::of(&Some(3), None).preview, "Some(3)");
    assert_eq!(Node::of(&None::<u8>, None).preview, "None");
}

#[test]
fn a_long_preview_is_cut_at_its_budget() {
    let long: Vec<Process> = (0..1000).map(process).collect();

    let node = Node::of(&long, None);

    assert_eq!(node.form, Form::List { len: 1000 });
    assert!(node.preview.starts_with(r#"[Process { pid: 0, name: "p0" }, "#), "{}", node.preview);
    assert!(node.preview.ends_with('…'), "{}", node.preview);
    assert!(node.preview.len() <= PREVIEW + '…'.len_utf8(), "{}", node.preview.len());
}

#[test]
fn a_preview_reads_only_what_fits_in_it() {
    let asked = Cell::new(0);
    let long: Vec<Counted> = (0..10_000).map(|n| Counted { n, asked: &asked }).collect();

    Node::of(&long, None);

    assert!(asked.get() < 100, "read {} of 10000 for one line", asked.get());
}

#[test]
fn an_open_list_reads_only_its_window() {
    let asked = Cell::new(0);
    let long: Vec<Counted> = (0..10_000).map(|n| Counted { n, asked: &asked }).collect();
    let looking = Open {
        window: Some(5000..5010),
        children: Vec::new(),
    };

    let node = Node::of(&long, Some(&looking));

    let items = children(&node);
    assert_eq!(items.len(), 10);
    assert_eq!(items[0].0, Step::Index(5000));
    assert_eq!(items[0].1.preview, "5000");
    assert!(asked.get() < 150, "read {} of 10000 for ten", asked.get());
}

#[test]
fn a_list_opened_without_a_window_shows_the_first_ones() {
    let long: Vec<u32> = (0..1000).collect();

    let node = Node::of(&long, Some(&open()));

    assert_eq!(children(&node).len(), WINDOW);
}

#[test]
fn a_window_past_the_end_shows_what_there_is() {
    let short = vec![1, 2, 3];
    let looking = Open {
        window: Some(2..50),
        children: Vec::new(),
    };

    let node = Node::of(&short, Some(&looking));

    assert_eq!(children(&node).len(), 1);
}

#[test]
fn a_child_opens_where_its_path_says() {
    let state = vec![process(1), process(2), process(3)];

    let node = Node::of(&state, Some(&opened_at(&[Step::Index(2)])));

    let items = children(&node);
    assert_eq!(items[0].1.opened, None, "only the third was opened");
    let third = children(&items[2].1);
    assert_eq!(third[0].1.preview, "3");
}

#[test]
fn a_map_entry_is_found_by_its_key() {
    let mut map = BTreeMap::new();
    map.insert("a".to_string(), process(1));
    map.insert("b".to_string(), process(2));

    let node = Node::of(&map, Some(&opened_at(&[Step::Key(r#""b""#.to_string())])));

    assert_eq!(node.form, Form::Map { len: 2 });
    assert_eq!(node.preview, r#"{"a": Process { pid: 1, name: "p1" }, "b": Process { pid: 2, name: "p2" }}"#);
    let entries = children(&node);
    assert_eq!(entries[1].0, Step::Key(r#""b""#.to_string()));
    assert_eq!(entries[0].1.opened, None);
    assert_eq!(children(&entries[1].1)[0].1.preview, "2");
}

#[derive(Debug)]
#[allow(dead_code)]
struct OnlyDebug {
    items: Vec<u32>,
}

#[test]
fn what_only_debug_can_show_is_previewed_and_opened_as_text() {
    let state = OnlyDebug { items: vec![1, 2] };
    let show = |at: Inspector<'_>| at.debug(&state);

    let node = Node::shown(show, Some(&open()));

    assert_eq!(node.form, Form::Text);
    assert_eq!(node.preview, "OnlyDebug { items: [1, 2] }");
    assert_eq!(node.opened, Some(Opened::Text(format!("{state:#?}"))));
}

/// Writes for as long as it is let.
struct Endless;

impl fmt::Debug for Endless {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        loop {
            f.write_str("more ")?;
        }
    }
}

#[test]
fn debug_text_stops_at_its_budget_whatever_the_value() {
    let node = Node::shown(|at| at.debug(&Endless), Some(&open()));

    assert!(node.preview.len() <= PREVIEW + '…'.len_utf8());
    let Some(Opened::Text(text)) = node.opened else {
        panic!("not opened as text");
    };
    assert!(text.len() <= TEXT + '…'.len_utf8(), "{}", text.len());
    assert!(text.ends_with('…'));
}

#[derive(Debug)]
struct Both;

impl Inspect for Both {
    fn inspect(&self, at: Inspector<'_>) {
        at.unit("by Inspect");
    }
}

#[derive(Debug)]
struct DebugOnly;

struct Neither;

#[test]
fn a_type_is_shown_by_its_inspect_before_its_debug_and_by_its_name_without_either() {
    let by_inspect = Node::shown(|at| (&&&Probe(&Both)).show(at), None);
    let by_debug = Node::shown(|at| (&&&Probe(&DebugOnly)).show(at), None);
    let by_name = Node::shown(|at| (&&&Probe(&Neither)).show(at), None);

    assert_eq!(by_inspect.preview, "by Inspect");
    assert_eq!(by_debug.preview, "DebugOnly");
    assert_eq!(by_debug.form, Form::Text);
    assert!(by_name.preview.ends_with("Neither"), "{}", by_name.preview);
    assert_eq!(by_name.form, Form::Opaque);
}
