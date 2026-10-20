use std::time::Duration;

use guinea::app::Harness;
use guinea::prelude::*;

mod tasks {
    use super::*;

    //@show features: state and updates
    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Tasks(pub Vec<String>);

    #[derive(Clone)]
    pub enum Change {
        Added(String),
        Cleared,
    }

    #[reducer]
    fn tasks(this: &mut Tasks, change: Change) {
        match change {
            Change::Added(title) => this.0.push(title),
            Change::Cleared => this.0.clear(),
        }
    }
    //@show-end

    //@show features: actions
    pub struct AddTask(pub String);
    pub struct ClearTasks;
    //@show-end

    //@show features: an actor decides
    #[derive(Debug)]
    struct Editing {
        push: Push<Tasks>,
    }

    actor! {
        Editing {
            handlers { AddTask, ClearTasks }
        }
    }

    #[handler]
    fn add(In(this, AddTask(title), _): In<Editing, AddTask>) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }

        this.push.send(Change::Added(title.to_owned()));
    }

    #[handler]
    fn clear(In(this, _, _): In<Editing, ClearTasks>) {
        this.push.send(Change::Cleared);
    }
    //@show-end

    //@show features: installation
    feature! {
        pub TaskList {
            exports { Tasks }
        }
    }

    #[installs]
    fn task_list(cx: &Install) -> anyhow::Result<TaskList> {
        let (tasks, _) = cx.state::<Tasks>().driven_by(|push| Editing { push });
        Ok(TaskList(tasks))
    }
    //@show-end
}

use tasks::{AddTask, ClearTasks, TaskList, Tasks};

//@show features: actions become state
#[guinea::test]
fn a_task_gets_a_title_before_it_is_added(h: &mut Harness) {
    h.install::<TaskList>(&()).unwrap();
    let dispatch = h.dispatch::<Tasks>();

    dispatch.emit(AddTask("  Read the book  ".into()));
    dispatch.emit(AddTask("   ".into()));
    h.settled();

    assert_eq!(h.state::<Tasks>().0, vec!["Read the book"]);

    dispatch.emit(ClearTasks);
    h.settled();

    assert!(h.state::<Tasks>().0.is_empty());
}
//@show-end

//@show features: separate instances
#[guinea::test]
fn two_pages_can_have_separate_task_lists(h: &mut Harness) {
    let work = h.child();
    let home = h.child();
    work.install::<TaskList>(&()).unwrap();
    home.install::<TaskList>(&()).unwrap();

    work.dispatch::<Tasks>()
        .emit(AddTask("Review a change".into()));
    home.dispatch::<Tasks>().emit(AddTask("Buy bread".into()));
    h.settled();

    assert_eq!(work.state::<Tasks>().0, vec!["Review a change"]);
    assert_eq!(home.state::<Tasks>().0, vec!["Buy bread"]);
}
//@show-end

//@show loading: state
#[derive(Clone, PartialEq, Debug)]
pub struct Listing {
    pub rows: Load<Vec<String>>,
}

impl Default for Listing {
    fn default() -> Self {
        Self {
            rows: Load::NotAsked,
        }
    }
}

#[reducer]
fn listing(this: &mut Listing, rows: Load<Vec<String>>) {
    this.rows = rows;
}
//@show-end

//@show loading: a loader
pub struct Refresh {
    pub simulate_failure: bool,
}

async fn fetch(simulate_failure: bool) -> anyhow::Result<Vec<String>> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    anyhow::ensure!(!simulate_failure, "service unavailable");
    Ok(vec!["guinea".into(), "shown".into()])
}

#[derive(Debug)]
struct Loader {
    push: Push<Listing>,
}

actor! {
    Loader {
        handlers { Refresh }
    }
}

#[handler]
fn refresh(In(this, Refresh { simulate_failure }, cx): In<Loader, Refresh>) {
    this.push.send(Load::Loading);
    cx.task().latest().pushing(this.push).spawn(async move {
        match fetch(simulate_failure).await {
            Ok(rows) => Load::Ready(rows),
            Err(error) => Load::failed(error),
        }
    });
}

feature! {
    pub Catalogue {
        exports { Listing }
    }
}

#[installs]
fn catalogue(cx: &Install) -> anyhow::Result<Catalogue> {
    let (listing, _) = cx.state::<Listing>().driven_by(|push| Loader { push });
    Ok(Catalogue(listing))
}
//@show-end

//@show loading: presenting each state
fn status(listing: &Listing) -> String {
    match &listing.rows {
        Load::NotAsked => "Load the catalogue".into(),
        Load::Loading => "Loading…".into(),
        Load::Ready(rows) => format!("{} items", rows.len()),
        Load::Failed(error) => format!("Could not load: {error}"),
    }
}
//@show-end

//@show loading: successful loading
#[guinea::test]
fn a_request_moves_from_not_asked_through_loading_to_ready(h: &mut Harness) {
    h.install::<Catalogue>(&()).unwrap();
    assert!(matches!(h.state::<Listing>().rows, Load::NotAsked));
    assert_eq!(status(&h.state::<Listing>()), "Load the catalogue");

    h.dispatch::<Listing>().emit(Refresh {
        simulate_failure: false,
    });
    h.settled();
    assert!(h.state::<Listing>().rows.is_loading());
    assert_eq!(status(&h.state::<Listing>()), "Loading…");

    h.advance(Duration::from_secs(1));
    assert_eq!(
        h.state::<Listing>().rows.ready().unwrap(),
        &["guinea", "shown"]
    );
    assert_eq!(status(&h.state::<Listing>()), "2 items");
}
//@show-end

//@show loading: failure and retry
#[guinea::test]
fn a_failed_load_can_be_retried(h: &mut Harness) {
    h.install::<Catalogue>(&()).unwrap();
    let dispatch = h.dispatch::<Listing>();
    dispatch.emit(Refresh {
        simulate_failure: true,
    });
    h.settled();
    h.advance(Duration::from_secs(1));

    let failed = h.state::<Listing>();
    assert!(failed.rows.ready().is_none());
    assert_eq!(
        failed.rows.error().unwrap().to_string(),
        "service unavailable"
    );
    assert_eq!(status(&failed), "Could not load: service unavailable");

    dispatch.emit(Refresh {
        simulate_failure: false,
    });
    h.settled();
    assert!(h.state::<Listing>().rows.is_loading());
    assert!(h.state::<Listing>().rows.error().is_none());
    h.advance(Duration::from_secs(1));
    assert_eq!(status(&h.state::<Listing>()), "2 items");
}
//@show-end

#[guinea::test]
fn an_old_failed_request_cannot_replace_a_new_load(h: &mut Harness) {
    h.install::<Catalogue>(&()).unwrap();
    let dispatch = h.dispatch::<Listing>();
    dispatch.emit(Refresh {
        simulate_failure: true,
    });
    h.settled();
    h.advance(Duration::from_millis(500));
    dispatch.emit(Refresh {
        simulate_failure: false,
    });
    h.settled();
    h.advance(Duration::from_millis(500));
    assert!(h.state::<Listing>().rows.is_loading());

    h.advance(Duration::from_millis(500));
    assert_eq!(status(&h.state::<Listing>()), "2 items");
}

#[test]
fn default_loading_and_an_empty_ready_value_have_different_meanings() {
    let loading = Load::<Vec<String>>::default();
    assert!(loading.is_loading());
    assert!(loading.ready().is_none());

    let ready = Load::Ready(Vec::<String>::new());
    assert!(!ready.is_loading());
    assert!(ready.ready().unwrap().is_empty());
    assert_eq!(status(&Listing { rows: ready }), "0 items");
}

#[test]
fn cloned_failures_share_identity_but_a_new_failure_is_a_new_value() {
    let failed = Load::<()>::failed(anyhow::anyhow!("service unavailable"));
    assert_eq!(failed, failed.clone());
    assert_ne!(failed, Load::failed(anyhow::anyhow!("service unavailable")));
}
