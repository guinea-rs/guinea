//! What `#[derive(Inspect)]` writes, read back through a [`Node`].

use std::rc::Rc;

use guinea_core::__private::Home;
use guinea_core::actor::{Addr, Handler, In};
use guinea_core::inspect::{Form, Node, Open, Opened, Step};
use guinea_core::scope::ScopeTree;
use guinea_macros::{Inspect, actor, reducer};

#[derive(Inspect)]
struct Process {
    pid: u32,
    name: String,
    #[inspect(skip)]
    #[allow(dead_code)]
    kill: Rc<dyn Fn()>,
}

#[derive(Inspect)]
struct Pid(u32, &'static str);

#[derive(Inspect)]
struct Nothing;

#[derive(Inspect)]
enum Status {
    Running { since: u64 },
    Exited(i32, #[inspect(skip)] ()),
    Unknown,
}

#[derive(Inspect)]
struct Page<T> {
    rows: Vec<T>,
}

#[derive(Inspect)]
struct Visit {
    at: u64,
    path: String,
}

#[derive(Inspect)]
struct Held<T> {
    count: u32,
    #[inspect(skip)]
    #[allow(dead_code)]
    value: T,
}

#[derive(Debug, Inspect)]
struct Mount {
    r#type: String,
    size: u32,
}

fn children(node: &Node) -> &[(Step, Node)] {
    match &node.opened {
        Some(Opened::Children(children)) => children,
        other => panic!("not opened into children: {other:?}"),
    }
}

#[test]
fn a_struct_shows_its_fields_by_name_and_leaves_out_the_skipped() {
    let process = Process {
        pid: 4,
        name: "System".into(),
        kill: Rc::new(|| {}),
    };

    let node = Node::of(&process, Some(&Open::default()));

    assert_eq!(node.form, Form::Record);
    assert_eq!(node.preview, r#"Process { pid: 4, name: "System" }"#);
    let fields: Vec<_> = children(&node).iter().map(|(step, _)| step.clone()).collect();
    assert_eq!(
        fields,
        [Step::Field("pid".into()), Step::Field("name".into())]
    );
}

#[test]
fn a_tuple_struct_and_a_unit_struct_show_as_debug_would() {
    assert_eq!(Node::of(&Pid(4, "x"), None).preview, r#"Pid(4, "x")"#);
    assert_eq!(Node::of(&Nothing, None).preview, "Nothing");
}

#[test]
fn a_field_named_at_is_shown_like_any_other() {
    let visit = Visit {
        at: 7,
        path: "/".into(),
    };

    assert_eq!(Node::of(&visit, None).preview, r#"Visit { at: 7, path: "/" }"#);
}

#[test]
fn a_raw_field_is_named_as_its_debug_names_it() {
    let mount = Mount {
        r#type: "ntfs".into(),
        size: 4,
    };

    let node = Node::of(&mount, Some(&Open::default()));

    assert_eq!(node.preview, format!("{mount:?}"));
    assert_eq!(children(&node)[0].0, Step::Field("type".into()));
}

#[test]
fn an_enum_shows_the_variant_it_is() {
    assert_eq!(
        Node::of(&Status::Running { since: 7 }, None).preview,
        "Running { since: 7 }"
    );
    assert_eq!(Node::of(&Status::Exited(1, ()), None).preview, "Exited(1)");
    assert_eq!(Node::of(&Status::Unknown, None).preview, "Unknown");
}

struct Client;

struct Refresh;

#[derive(Inspect)]
struct Lister {
    rows: Vec<u32>,
    #[inspect(skip)]
    #[allow(dead_code)]
    client: Client,
}

actor! {
    Lister {
        handlers { Refresh }
    }
}

impl Handler<Refresh> for Lister {
    fn handle(_: In<Self, Refresh>) {}
}

#[derive(Debug)]
#[allow(dead_code)]
struct Printed(u32);

actor! {
    Printed {
        handlers { Refresh }
    }
}

impl Handler<Refresh> for Printed {
    fn handle(_: In<Self, Refresh>) {}
}

struct Silent;

actor! {
    Silent {
        handlers { Refresh }
    }
}

impl Handler<Refresh> for Silent {
    fn handle(_: In<Self, Refresh>) {}
}

fn rows_from(window: std::ops::Range<usize>) -> Open {
    Open {
        window: None,
        children: vec![(
            Step::Field("rows".into()),
            Open {
                window: Some(window),
                children: Vec::new(),
            },
        )],
    }
}

#[test]
fn an_actor_is_listed_by_its_preview_and_opened_where_asked() {
    let tree = ScopeTree::new();
    let rows = (0..1000).collect();
    let id = Addr::new(Lister { rows, client: Client }, &Home::new(tree.scope())).id();

    let listed = tree.actor(id).expect("listed");
    assert_eq!(listed.state.form, Form::Record);
    assert!(listed.state.preview.starts_with("Lister { rows: [0, 1, 2, "), "{}", listed.state.preview);
    assert_eq!(listed.state.opened, None);

    let opened = tree.inspect_actor(id, &rows_from(500..503)).expect("there");
    let rows = &children(&opened)[0].1;
    assert_eq!(rows.form, Form::List { len: 1000 });
    let shown: Vec<_> = children(rows).iter().map(|(_, row)| row.preview.as_str()).collect();
    assert_eq!(shown, ["500", "501", "502"]);
}

#[test]
fn an_actor_without_inspect_is_shown_by_its_debug_and_without_either_by_its_name() {
    let tree = ScopeTree::new();
    let home = Home::new(tree.scope());

    let printed = Addr::new(Printed(7), &home).inspect(None);
    let silent = Addr::new(Silent, &home).inspect(None);

    assert_eq!(printed.form, Form::Text);
    assert_eq!(printed.preview, "Printed(7)");
    assert_eq!(silent.form, Form::Opaque);
    assert!(silent.preview.ends_with("Silent"), "{}", silent.preview);
}

#[derive(Clone, Default, Inspect)]
struct Rows {
    rows: Vec<u32>,
}

#[reducer]
fn rows(this: &mut Rows, rows: Vec<u32>) {
    this.rows = rows;
}

#[derive(Clone, Default)]
struct Bare;

#[reducer]
fn bare(_this: &mut Bare, _: ()) {}

#[test]
fn a_reducer_is_described_by_its_preview_and_opened_where_asked() {
    let tree = ScopeTree::new();
    let scope = tree.scope();
    scope.state::<Rows>().replace(Rc::new(Rows {
        rows: (0..1000).collect(),
    }));
    scope.state::<Bare>();

    let described = scope.describe_states();
    let rows = described.iter().find(|state| state.type_name.ends_with("Rows")).expect("Rows");
    assert!(rows.state.preview.starts_with("Rows { rows: [0, 1, "), "{}", rows.state.preview);
    let bare = described.iter().find(|state| state.type_name.ends_with("Bare")).expect("Bare");
    assert_eq!(bare.state.form, Form::Opaque);

    let opened = scope
        .inspect_state(rows.type_name, &rows_from(998..1000))
        .expect("there");
    let shown: Vec<_> = children(&children(&opened)[0].1)
        .iter()
        .map(|(_, row)| row.preview.as_str())
        .collect();
    assert_eq!(shown, ["998", "999"]);
}

#[derive(Inspect)]
struct Pushing {
    push: guinea_core::feature::Push<Rows>,
    dispatch: guinea_core::feature::Dispatch,
}

#[test]
fn a_handle_shows_what_it_is_a_handle_to() {
    let tree = ScopeTree::new();
    let pushing = Pushing {
        push: guinea_core::feature::Push::new(tree.scope()),
        dispatch: guinea_core::feature::Dispatch::nowhere(),
    };

    let node = Node::of(&pushing, Some(&Open::default()));

    let fields = children(&node);
    assert_eq!(fields[0].1.form, Form::Value);
    let push = &fields[0].1.preview;
    assert!(push.starts_with("Push<") && push.ends_with("Rows>"), "{push}");
    assert_eq!(fields[1].1.preview, "Dispatch");
}

#[test]
fn a_type_parameter_only_a_skipped_field_holds_need_not_be_inspect() {
    let held = Held {
        count: 2,
        value: Client,
    };

    assert_eq!(Node::of(&held, None).preview, "Held { count: 2 }");
}

#[test]
fn a_generic_struct_shows_what_it_holds() {
    let page = Page {
        rows: vec![Pid(1, "a")],
    };

    assert_eq!(Node::of(&page, None).preview, r#"Page { rows: [Pid(1, "a")] }"#);
}
