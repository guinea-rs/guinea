use super::*;
use crate::{Layout, LayoutCx, Page, PageCx, Routable, layout, page};
use windows_reactor::{EncodedImage, Image, StackPanel};

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
        let navigation = cx.navigate::<Route>();
        cx.outlet(&navigation, |route, cx| match route {
            Route::First => cx.mount::<First>(&()),
            Route::Second => cx.mount::<Second>(&()),
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
