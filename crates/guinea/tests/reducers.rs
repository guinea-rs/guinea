//! The examples in `#[reducer]`'s documentation, run. `cargo xtask docs`
//! copies what is between the marks into it.

use guinea::app::Harness;
use guinea::prelude::*;

mod counting {
    use super::*;

    //@show a reducer
    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Count(pub u32);

    #[reducer]
    fn count(this: &mut Count, by: u32) {
        this.0 += by;
    }
    //@show-end

    pub struct Add(pub u32);

    #[derive(Debug)]
    pub struct Counting {
        push: Push<Count>,
    }

    actor! {
        Counting {
            handlers { Add }
        }
    }

    #[handler]
    fn add(this: &mut Counting, Add(by): Add) {
        this.push.send(by);
    }

    feature! {
        pub Counter {
            exports { Count }
        }
    }

    #[installs]
    fn counter(cx: &FeatureInitContext) -> anyhow::Result<Counter> {
        let (count, _) = cx.state::<Count>().driven_by(|push| Counting { push });
        Ok(Counter(count))
    }

    #[guinea::test(iterations = 2)]
    fn what_is_pushed_is_reduced(h: &mut Harness) {
        h.install::<Counter>(&()).unwrap();
        h.dispatch::<Count>().emit(Add(2));
        h.dispatch::<Count>().emit(Add(3));
        h.settled();

        assert_eq!(h.state::<Count>().0, 5);
    }
}

mod sorting {
    use super::*;

    //@show a reducer of an enum
    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Table {
        pub rows: Vec<String>,
        pub descending: bool,
    }

    #[derive(Clone, Debug)]
    pub enum Changed {
        Rows(Vec<String>),
        Sorted { descending: bool },
    }

    #[reducer]
    fn table(this: &mut Table, changed: Changed) {
        match changed {
            Changed::Rows(rows) => this.rows = rows,
            Changed::Sorted { descending } => this.descending = descending,
        }
    }
    //@show-end

    pub struct Sort;

    #[derive(Debug)]
    pub struct Sorting {
        push: Push<Table>,
    }

    actor! {
        Sorting {
            handlers { Sort }
        }
    }

    #[handler]
    fn sort(this: &mut Sorting, _: Sort) {
        this.push.send(Changed::Rows(vec!["b".into(), "a".into()]));
        this.push.send(Changed::Sorted { descending: true });
    }

    feature! {
        pub Tables {
            exports { Table }
        }
    }

    #[installs]
    fn tables(cx: &FeatureInitContext) -> anyhow::Result<Tables> {
        let (table, _) = cx.state::<Table>().driven_by(|push| Sorting { push });
        Ok(Tables(table))
    }

    #[guinea::test(iterations = 2)]
    fn each_shape_changes_its_own_field(h: &mut Harness) {
        h.install::<Tables>(&()).unwrap();
        h.dispatch::<Table>().emit(Sort);
        h.settled();

        let table = h.state::<Table>();
        assert_eq!(table.rows, ["b", "a"]);
        assert!(table.descending);
    }
}
