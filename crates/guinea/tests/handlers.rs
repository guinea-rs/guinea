//! The examples in `#[handler]`'s documentation, run. `cargo xtask docs`
//! copies what is between the marks into it.

use guinea::app::Harness;
use guinea::prelude::*;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

impl Reducer for Count {
    type Update = u32;

    fn reduce(&mut self, by: u32) {
        self.0 += by;
    }
}

mod message_only {
    use super::*;

    //@show a handler that takes the message
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
    //@show-end

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
    fn adds_what_it_was_given(h: &mut Harness) {
        h.install::<Counter>(&()).unwrap();
        h.dispatch::<Count>().emit(Add(3));
        h.settled();

        assert_eq!(h.state::<Count>().0, 3);
    }
}

mod with_cx {
    use super::*;

    //@show a handler that starts work
    pub struct Refresh;
    pub struct Counted(pub u32);

    #[derive(Debug)]
    pub struct Counting {
        push: Push<Count>,
    }

    actor! {
        Counting {
            handlers { Refresh => { bg Counted }, Counted }
        }
    }

    // `cx` is `Cx<Counting, Refresh>`: the macro writes in what the two
    // arguments before it already say.
    #[handler]
    fn refresh(_this: &mut Counting, _: Refresh, cx: Cx) {
        cx.spawn_bg::<Counted, _>(async {
            guinea::core::executor::random_delay().await;
            Counted(7)
        });
    }

    #[handler]
    fn counted(this: &mut Counting, Counted(n): Counted) {
        this.push.send(n);
    }
    //@show-end

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

    #[guinea::test(iterations = 8)]
    fn what_the_work_found_comes_back_to_the_actor(h: &mut Harness) {
        h.install::<Counter>(&()).unwrap();
        h.dispatch::<Count>().emit(Refresh);
        h.settled();

        assert_eq!(h.state::<Count>().0, 7);
    }
}

mod asynchronous {
    use super::*;

    //@show an async handler
    pub struct Load(pub u32);
    pub struct Loaded(pub u32);

    #[derive(Debug)]
    pub struct Loader {
        push: Push<Count>,
    }

    actor! {
        Loader {
            handlers { Load, Loaded }
        }
    }

    // Runs off the UI thread, and ends with the actor: `cx` knows when it is
    // gone, and sends back to it while it is not.
    #[handler]
    async fn load(cx: AsyncContext<Loader>, Load(n): Load) {
        guinea::core::executor::random_delay().await;
        cx.send(Loaded(n * 2));
    }

    #[handler]
    fn loaded(this: &mut Loader, Loaded(n): Loaded) {
        this.push.send(n);
    }
    //@show-end

    feature! {
        pub Loading {
            exports { Count }
        }
    }

    #[installs]
    fn loading(cx: &FeatureInitContext) -> anyhow::Result<Loading> {
        let (count, _) = cx.state::<Count>().driven_by(|push| Loader { push });
        Ok(Loading(count))
    }

    #[guinea::test(iterations = 8)]
    fn the_body_sends_its_answer_back(h: &mut Harness) {
        h.install::<Loading>(&()).unwrap();
        h.dispatch::<Count>().emit(Load(4));
        h.settled();

        assert_eq!(h.state::<Count>().0, 8);
    }
}
