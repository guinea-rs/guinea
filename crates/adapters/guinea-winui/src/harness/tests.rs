use super::*;
use crate::{Layout, LayoutCx, Page, PageCx, Routable, layout, page};
use windows_reactor::{EncodedImage, Image, StackPanel};

thread_local! {
    static OWNED_INSTALLS: RefCell<Vec<String>> = RefCell::default();
}

#[derive(Clone, Debug, PartialEq, Routable)]
enum OwnedRoute { Details { name: String }, Empty }

#[derive(Default)]
struct OwnedShell;

#[layout]
impl Layout for OwnedShell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        cx.outlet::<OwnedRoute>(|route| match route {
            OwnedRoute::Details { name } => crate::mount::<OwnedPage>(name.clone()).keep(),
            OwnedRoute::Empty => crate::mount::<First>(()),
        })
    }
}

struct OwnedPage(String);

#[page]
impl Page for OwnedPage {
    type Params = String;
    fn install(cx: &crate::Install<String>) -> anyhow::Result<Self> {
        OWNED_INSTALLS.with_borrow_mut(|names| names.push(cx.params().clone()));
        Ok(Self(cx.params().clone()))
    }
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        windows_reactor::TextBlock::new().text(&self.0).into()
    }
}

#[test]
fn deferred_mount_owns_params_and_keeps_the_same_route_instance() {
    guinea_app::app::check(4, |h| {
        OWNED_INSTALLS.with_borrow_mut(Vec::clear);
        let declaration = crate::mount::<OwnedPage>(String::from("unused"));
        OWNED_INSTALLS.with_borrow(|names| assert!(names.is_empty()));
        drop(declaration);
        let mut page = Mounted::routed::<OwnedShell>(h, OwnedRoute::Details { name: "first".into() }).unwrap();
        let nav = page.navigator::<OwnedRoute>();
        nav.to(OwnedRoute::Empty);
        page.settle();
        nav.to(OwnedRoute::Details { name: "first".into() });
        page.settle();
        OWNED_INSTALLS.with_borrow(|names| assert_eq!(names, &["first"]));
        nav.to(OwnedRoute::Details { name: "second".into() });
        page.settle();
        OWNED_INSTALLS.with_borrow(|names| assert_eq!(names, &["first", "second"]));
    });
}

#[derive(Clone, Debug, PartialEq, Routable)]
enum Route {
    First,
    Second,
}

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        cx.outlet::<Route>(|route| match route {
            Route::First => crate::mount::<First>(()),
            Route::Second => crate::mount::<Second>(()),
        })
    }
}

#[derive(Default)]
struct First;

#[page]
impl Page for First {
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        images(1)
    }
}

#[derive(Default)]
struct Second;

#[page]
impl Page for Second {
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        images(27)
    }
}

fn images(count: usize) -> View {
    StackPanel::new()
        .children(
            (0..count)
                .map(|_| {
                    Image::new()
                        .source_data(EncodedImage::from_static(b"recording-only image"))
                        .into()
                })
                .collect::<Vec<View>>(),
        )
        .into()
}

fn image_ids(host: &ComponentHost<RecordingAdapter>) -> Vec<NodeId> {
    let graph = host.runtime().graph();
    graph
        .objects()
        .filter(|object| graph.kind(*object) == Some(ObjectType::Image))
        .collect()
}

#[test]
fn routing_keeps_the_shared_image_in_every_batch_even_with_page_refreshes() {
    guinea_app::app::check(32, |harness| {
        let mut mounted = Mounted::routed::<Shell>(harness, Route::First).unwrap();
        let first = image_ids(&mounted.host)[0];
        let mut live = std::collections::HashSet::from([first]);
        let navigation = mounted.navigator::<Route>();

        for route in [Route::Second, Route::First, Route::Second, Route::First] {
            let from = mounted.host.adapter().batches().len();
            let refresh = match route {
                Route::Second => sender::<First, <First as Page>::Message>(mounted.owner),
                Route::First => sender::<Second, <Second as Page>::Message>(mounted.owner),
            };
            refresh(Signal::Refresh(None));
            navigation.to(route.clone());
            refresh(Signal::Refresh(None));
            mounted.settle();

            for batch in &mounted.host.adapter().batches()[from..] {
                for mutation in batch {
                    match mutation {
                        Mutation::Create {
                            object,
                            kind: ObjectType::Image,
                        } => {
                            live.insert(*object);
                        }
                        Mutation::Destroy { object } => {
                            assert_ne!(*object, first, "navigation destroyed the shared image");
                            live.remove(object);
                        }
                        Mutation::SetProperties { object, set, clear } if *object == first => {
                            assert!(!clear.contains(&PropertyId::Source));
                            assert!(set.iter().all(|property| property.id != PropertyId::Source));
                        }
                        _ => {}
                    }
                }
                assert!(live.contains(&first), "an update removed the shared image");
                assert!(!live.is_empty(), "an update inserted an empty image tree");
            }
            let expected = if route == Route::First { 1 } else { 27 };
            assert_eq!(live.len(), expected);
            assert_eq!(image_ids(&mounted.host).len(), expected);
        }
    });
}
