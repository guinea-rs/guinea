use std::cell::Cell;
use std::rc::Rc;

use guinea_core::scope::{Scope, ScopeTree};
use guinea_router::mount::Pass;

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

    assert_eq!(failed.map_err(|error| error.to_string()), Err("no connection".to_string()));
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
