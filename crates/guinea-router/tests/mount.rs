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
