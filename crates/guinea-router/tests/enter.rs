//! What an enter guard takes to decide.

use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea_core::guard::Verdict;
use guinea_core::scope::ScopeTree;
use guinea_macros::given;
use guinea_router::enter::{Enter, EnterCx};

#[derive(Clone)]
struct Session {
    admin: bool,
}

struct RequiresAdmin;

impl Enter for RequiresAdmin {
    #[given]
    fn decide(_cx: &EnterCx<'_>, session: Option<Session>) -> Verdict {
        match session {
            Some(session) if session.admin => Verdict::Allow,
            _ => Verdict::Block,
        }
    }
}

struct RequiresSession;

impl Enter for RequiresSession {
    #[given]
    fn decide(_cx: &EnterCx<'_>, _session: Session) -> Verdict {
        Verdict::Allow
    }
}

#[test]
fn a_guard_decides_on_what_it_takes_and_on_its_absence() {
    let tree = ScopeTree::new();
    let window = tree.child();
    let blocked = matches!(RequiresAdmin::decide(&EnterCx::new(window, "/admin")), Verdict::Block);

    tree.provide(Session { admin: true }, "test").unwrap();
    let allowed = matches!(RequiresAdmin::decide(&EnterCx::new(window, "/admin")), Verdict::Allow);

    assert_eq!((blocked, allowed), (true, true));
}

#[test]
fn a_guard_without_what_it_cannot_do_without_panics_naming_it() {
    let tree = ScopeTree::new();

    let decided = catch_unwind(AssertUnwindSafe(|| {
        RequiresSession::decide(&EnterCx::new(tree.child(), "/audit"));
    }));
    let said = decided
        .err()
        .and_then(|panic| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default();

    assert!(said.contains("RequiresSession") && said.contains("Session"), "{said}");
}
