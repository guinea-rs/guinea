use guinea_core::guard::Verdict;
use guinea_core::scope::ScopeTree;
use guinea_router::enter::{Enter, EnterCx};
use guinea_router::navigator::{Navigator, Route};

struct Signed;
struct Admin;

impl Enter for Signed {
    fn decide(_cx: &EnterCx<'_>) -> Verdict {
        Verdict::Allow
    }
}

impl Enter for Admin {
    fn decide(_cx: &EnterCx<'_>) -> Verdict {
        Verdict::Block
    }
}

#[derive(Clone, Debug, PartialEq, guinea_macros::Route)]
#[guard(Signed)]
enum Top {
    #[at("/processes")]
    Processes(Processes),
    #[at("/process/:pid")]
    Process { pid: u32 },
    #[at("/process/new")]
    NewProcess,
    #[guard(Admin)]
    #[at("/admin")]
    Admin(Settings),
    Unlinked,
}

#[derive(Clone, Debug, PartialEq, guinea_macros::Route)]
enum Processes {
    #[at("/")]
    List,
    #[at("/settings")]
    Settings,
}

#[derive(Clone, Debug, PartialEq, guinea_macros::Route)]
enum Settings {
    #[at("/")]
    General,
}

fn guards(route: &Top) -> Vec<&'static str> {
    route.guards().iter().map(|guard| guard.name()).collect()
}

#[test]
fn a_variant_is_named_and_a_branch_stays_while_only_its_level_below_moves() {
    assert_eq!(Top::Processes(Processes::List).name(), "Processes");
    assert_eq!(Top::Process { pid: 4 }.name(), "Process");

    assert!(Top::Processes(Processes::List).stays(&Top::Processes(Processes::Settings)));
    assert!(!Top::Processes(Processes::List).stays(&Top::NewProcess));
    assert!(!Top::Process { pid: 4 }.stays(&Top::Process { pid: 5 }), "a leaf with other params is left");
    assert!(Top::Process { pid: 4 }.stays(&Top::Process { pid: 4 }));
}

#[test]
fn guards_cascade_from_the_enum_down_through_a_branch() {
    assert_eq!(guards(&Top::NewProcess), ["Signed"]);
    assert_eq!(guards(&Top::Admin(Settings::General)), ["Signed", "Admin"]);
}

#[test]
fn a_nested_level_is_found_through_the_navigator_of_the_enum_around_it() {
    let tree = ScopeTree::new();
    Navigator::open(tree.scope(), Top::Admin(Settings::General));

    let nested = Navigator::<Settings>::find(tree.scope()).and_then(|found| found.current());

    assert_eq!(nested, Some(Settings::General));
}

#[test]
fn a_route_is_written_as_its_link_and_read_back_from_it() {
    let routes = [
        (Top::Processes(Processes::List), "/processes"),
        (Top::Processes(Processes::Settings), "/processes/settings"),
        (Top::Process { pid: 42 }, "/process/42"),
        (Top::NewProcess, "/process/new"),
        (Top::Admin(Settings::General), "/admin"),
    ];

    for (route, link) in routes {
        assert_eq!(route.link().as_deref(), Some(link), "{route:?}");
        assert_eq!(Top::parse(link), Some(route), "{link}");
    }
}

#[test]
fn what_no_link_reaches_is_not_read() {
    assert_eq!(Top::Unlinked.link(), None);
    assert_eq!(Top::parse("/process/notanumber"), None);
    assert_eq!(Top::parse("/nowhere"), None);
    assert_eq!(Top::parse("/processes/settings/more"), None);
}
