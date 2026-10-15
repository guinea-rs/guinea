use std::cell::Cell;
use std::rc::Rc;

use guinea_core::scope::{Scope, ScopeTree};
use guinea_router::mount::{Outlet, Pass};
use guinea_router::navigator::Navigator;

struct Listing;
struct Details;

fn counting(installs: &Rc<Cell<u32>>) -> impl FnOnce(&guinea_app::feature::FeatureInitContext) -> anyhow::Result<()> {
    let installs = installs.clone();
    move |_| {
        installs.set(installs.get() + 1);
        Ok(())
    }
}

fn drawn(parent: Scope, mounts: impl FnOnce(&Pass) -> Vec<Scope>) -> Vec<Scope> {
    let pass = Pass::begin(parent);
    let scopes = mounts(&pass);
    pass.end();
    scopes
}

#[test]
fn a_child_mounted_again_is_the_same_scope_installed_once() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let first = drawn(parent, |pass| vec![pass.mount::<Listing, _>(&(), counting(&installs)).unwrap()]);
    let second = drawn(parent, |pass| vec![pass.mount::<Listing, _>(&(), counting(&installs)).unwrap()]);

    assert_eq!(first, second);
    assert_eq!(first[0].parent(), Some(parent));
    assert_eq!(installs.get(), 1);
}

#[test]
fn other_params_are_another_child_and_the_old_one_goes_at_the_end_of_the_pass() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let one = drawn(parent, |pass| vec![pass.mount::<Details, _>(&1u32, counting(&installs)).unwrap()]);
    let two = drawn(parent, |pass| vec![pass.mount::<Details, _>(&2u32, counting(&installs)).unwrap()]);

    assert_ne!(one, two);
    assert_eq!(installs.get(), 2);
    assert!(!one[0].is_alive(), "not mounted in the second pass");
    assert!(two[0].is_alive());
}

#[test]
fn two_types_with_equal_params_are_two_children() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let both = drawn(parent, |pass| {
        vec![
            pass.mount::<Listing, _>(&(), counting(&installs)).unwrap(),
            pass.mount::<Details, _>(&(), counting(&installs)).unwrap(),
        ]
    });

    assert_ne!(both[0], both[1]);
    assert_eq!(installs.get(), 2);
}

#[test]
fn a_kept_child_left_out_sleeps_and_wakes_when_mounted_again() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let kept = drawn(parent, |pass| {
        let child = pass.mount::<Listing, _>(&(), counting(&installs)).unwrap();
        pass.keep(child);
        vec![child]
    });
    drawn(parent, |_| Vec::new());

    assert!(kept[0].is_alive(), "kept while left out");
    assert!(!kept[0].is_awake(), "asleep while left out");

    let again = drawn(parent, |pass| vec![pass.mount::<Listing, _>(&(), counting(&installs)).unwrap()]);

    assert_eq!(again, kept);
    assert!(again[0].is_awake());
    assert_eq!(installs.get(), 1, "woken, not installed again");
}

#[test]
fn a_child_that_fails_to_install_leaves_nothing_and_the_others_stand() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let standing = drawn(parent, |pass| vec![pass.mount::<Listing, _>(&(), counting(&installs)).unwrap()]);

    let pass = Pass::begin(parent);
    let listing = pass.mount::<Listing, _>(&(), counting(&installs)).unwrap();
    let failed = pass.mount::<Details, _>(&(), |_| anyhow::bail!("no connection"));
    pass.end();

    assert_eq!(
        failed.map_err(|error| format!("{error:#}")),
        Err("Details did not mount: no connection".to_string())
    );
    assert_eq!(listing, standing[0]);
    assert_eq!(parent.subtree().len(), 2, "the parent and its one child: the failed one's scope is gone");
}

#[test]
fn a_child_held_through_a_pass_that_did_not_mount_it_stands() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let standing = drawn(parent, |pass| vec![pass.mount::<Listing, _>(&(), counting(&installs)).unwrap()]);
    drawn(parent, |pass| {
        pass.hold(standing[0]);
        Vec::new()
    });

    assert!(standing[0].is_alive());
    assert!(standing[0].is_awake());
}

#[test]
fn what_a_parent_mounted_is_listed_in_order_by_type() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let both = drawn(parent, |pass| {
        vec![
            pass.mount::<Listing, _>(&(), counting(&installs)).unwrap(),
            pass.mount::<Details, _>(&(), counting(&installs)).unwrap(),
        ]
    });

    let names = |scope| guinea_router::mount::name(scope);
    assert_eq!(
        guinea_router::mount::mounted(parent),
        [(both[0], std::any::type_name::<Listing>()), (both[1], std::any::type_name::<Details>())]
    );
    assert_eq!(names(both[1]), Some(std::any::type_name::<Details>()));
    assert_eq!(names(parent), None, "nothing mounted the root");
}

#[test]
fn a_root_mounted_on_its_own_is_named_and_names_the_chain_below_it() {
    let tree = ScopeTree::new();
    let root = tree.scope();
    guinea_router::mount::root::<Listing>(root);

    let below = drawn(root, |pass| vec![pass.mount::<Details, _>(&(), |_| Ok(())).unwrap()]);

    let chain: Vec<_> = std::iter::successors(Some(below[0]), Scope::parent)
        .filter_map(guinea_router::mount::name)
        .collect();
    assert_eq!(chain, [std::any::type_name::<Details>(), std::any::type_name::<Listing>()]);
}

#[derive(Clone, Debug, PartialEq, guinea_macros::Routable)]
enum Place {
    Listing,
    Details,
}

fn outlet(parent: Scope, nav: &Navigator<Place>, broken: bool) -> Outlet {
    let pass = Pass::begin(parent);
    let shown = pass.outlet(nav, |place| match place {
        Place::Listing => pass.mount::<Listing, _>(&(), |_| Ok(())),
        Place::Details if broken => pass.mount::<Details, _>(&(), |_| anyhow::bail!("no connection")),
        Place::Details => pass.mount::<Details, _>(&(), |_| Ok(())),
    });
    pass.end();
    shown
}

#[test]
fn an_outlet_mounts_where_the_navigator_is_and_tells_the_navigator() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let nav = Navigator::open(parent, Place::Listing);

    let Outlet::Here(listing) = outlet(parent, &nav, false) else {
        panic!("the listing mounted");
    };

    assert_eq!(nav.drawn(parent), Some(listing));
    assert_eq!(guinea_router::mount::name(listing), Some(std::any::type_name::<Listing>()));
}

#[test]
fn a_place_that_fails_to_mount_keeps_what_stood_and_puts_the_navigator_back() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let nav = Navigator::open(parent, Place::Listing);
    let Outlet::Here(listing) = outlet(parent, &nav, true) else {
        panic!("the listing mounted");
    };

    nav.to(Place::Details);
    let shown = outlet(parent, &nav, true);

    assert!(matches!(shown, Outlet::Held(held) if held == listing), "{shown:?}");
    assert!(listing.is_alive(), "held through the pass");
    assert_eq!(nav.current(), Some(Place::Listing));
    assert_eq!(nav.failure().as_deref(), Some("Details did not mount: no connection"));
}

#[test]
fn a_first_place_that_fails_to_mount_says_why() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let nav = Navigator::open(parent, Place::Details);

    let shown = outlet(parent, &nav, true);

    let Outlet::Failed(error) = shown else {
        panic!("nothing stood, so the failure shows: {shown:?}");
    };
    assert_eq!(format!("{error:#}"), "Details did not mount: no connection");
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Hidden(u32);

#[guinea_macros::reducer]
fn hidden(this: &mut Hidden, to: u32) {
    this.0 = to;
}

struct Shell;
struct Prier;

fn said(run: impl FnOnce()) -> String {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
        .err()
        .and_then(|panic| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

fn keeping_hidden(parent: Scope) -> Scope {
    drawn(parent, |pass| {
        vec![
            pass.mount::<Shell, _>(&(), |cx| {
                cx.state::<Hidden>().seed(Hidden(1)).plain();
                Ok(())
            })
            .unwrap(),
        ]
    })[0]
}

#[test]
fn a_read_of_what_a_layout_kept_to_itself_names_the_chain_it_walked() {
    let tree = ScopeTree::new();
    let shell = keeping_hidden(tree.scope());
    let prier = drawn(shell, |pass| vec![pass.mount::<Prier, _>(&(), |_| Ok(())).unwrap()])[0];

    let said = said(|| {
        guinea_router::mount::binding::<Hidden>(prier);
    });

    assert!(said.contains("`Prier` reads `Hidden`, but nothing reaches it"), "{said}");
    assert!(said.contains("Prier <- Shell"), "{said}");
    assert!(said.contains("claimed by Shell, which does not list it in `Exports`"), "{said}");
}

#[test]
fn a_read_in_a_scope_that_was_removed_says_so() {
    let tree = ScopeTree::new();
    let shell = keeping_hidden(tree.scope());
    shell.remove();

    let said = said(|| {
        guinea_router::mount::binding::<Hidden>(shell);
    });

    assert!(said.contains("which was removed"), "{said}");
}

#[derive(Clone, Debug, guinea_macros::Event)]
struct Ping;

#[test]
fn a_kept_child_asleep_hears_nothing_and_is_not_told_on_waking() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let heard = Rc::new(Cell::new(0));
    let mount = |pass: &Pass| {
        let heard = heard.clone();
        let child = pass
            .mount::<Listing, _>(&(), move |cx| {
                cx.subscribe(move |_: Ping| heard.set(heard.get() + 1));
                Ok(())
            })
            .unwrap();
        pass.keep(child);
        child
    };

    drawn(parent, |pass| vec![mount(pass)]);
    drawn(parent, |_| Vec::new());
    parent.bus().publish(Ping);
    assert_eq!(heard.get(), 0, "it heard a ping in its sleep");

    drawn(parent, |pass| vec![mount(pass)]);
    assert_eq!(heard.get(), 0, "what it slept through came to it on waking");

    parent.bus().publish(Ping);
    assert_eq!(heard.get(), 1);
}

#[test]
fn a_kept_child_asleep_stays_asleep_when_its_type_mounts_with_other_params() {
    let tree = ScopeTree::new();
    let parent = tree.scope();
    let installs = Rc::new(Cell::new(0));

    let one = drawn(parent, |pass| {
        let child = pass.mount::<Details, _>(&1u32, counting(&installs)).unwrap();
        pass.keep(child);
        vec![child]
    })[0];
    drawn(parent, |_| Vec::new());
    let two = drawn(parent, |pass| vec![pass.mount::<Details, _>(&2u32, counting(&installs)).unwrap()])[0];

    assert_ne!(one, two);
    assert_eq!(installs.get(), 2);
    assert!(one.is_alive() && !one.is_awake(), "the kept one sleeps on beside it");
    assert!(two.is_awake());
}
