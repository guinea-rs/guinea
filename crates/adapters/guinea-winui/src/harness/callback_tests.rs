use super::*;
use crate::{
    Layout, LayoutCx, NavigatorExt, Page, PageCx, Routable, UpdateCx, Verdict, layout, page,
};
use windows_reactor::{Button, StackPanel, TextBlock};

// Intentionally not Clone: forwarding must move a widget's payload unchanged.
struct Payload(String);

#[derive(Default)]
struct ForwardPage {
    text: String,
}

#[page]
impl Page for ForwardPage {
    type Message = Payload;

    fn update(&mut self, Payload(text): Payload, _: &mut UpdateCx<'_, Self>) {
        self.text.push_str(&text);
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let forward: Callback<Payload> = cx.forward();
        StackPanel::new()
            .children((
                TextBlock::new().text(format!("page: {}", self.text)),
                Button::new()
                    .content("Forward to page")
                    .on_click(move || forward.call(Payload("x".into()))),
            ))
            .into()
    }
}

#[derive(Default)]
struct ForwardLayout {
    text: String,
}

#[layout]
impl Layout for ForwardLayout {
    type Message = Payload;

    fn update(&mut self, Payload(text): Payload, _: &mut UpdateCx<'_, Self>) {
        self.text.push_str(&text);
    }

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let forward: Callback<Payload> = cx.forward();
        StackPanel::new()
            .children((
                TextBlock::new().text(format!("layout: {}", self.text)),
                Button::new()
                    .content("Forward to layout")
                    .on_click(move || forward.call(Payload("y".into()))),
            ))
            .into()
    }
}

#[test]
fn page_forward_delivers_non_clone_messages_once_per_call() {
    guinea_app::app::check(4, |h| {
        let mut page = Mounted::<ForwardPage>::mount(h.segment(), ()).unwrap();
        page.click_text("Forward to page").settle();
        assert!(page.find_text("page: x").is_some());
        page.click_text("Forward to page").settle();
        assert!(page.find_text("page: xx").is_some());
    });
}

#[test]
fn layout_forward_delivers_non_clone_messages_once_per_call() {
    guinea_app::app::check(4, |h| {
        let mut layout = Mounted::<ForwardLayout>::mount(h.segment(), ()).unwrap();
        layout.click_text("Forward to layout").settle();
        assert!(layout.find_text("layout: y").is_some());
        layout.click_text("Forward to layout").settle();
        assert!(layout.find_text("layout: yy").is_some());
    });
}

#[derive(Clone, Debug, PartialEq, Routable)]
enum Route {
    Home,
    Details { name: String },
    Locked,
}

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Route>();
        StackPanel::new()
            .children((
                Button::new()
                    .content("Details")
                    .on_click(nav.to_callback(Route::Details {
                        name: "owned".into(),
                    })),
                cx.outlet::<Route>(|route| match route {
                    Route::Home => crate::mount::<ForwardPage>(()),
                    Route::Details { name } => crate::mount::<Details>(name.clone()),
                    Route::Locked => crate::mount::<Locked>(()),
                }),
            ))
            .into()
    }
}

struct Details(String);

#[page]
impl Page for Details {
    type Params = String;
    fn install(cx: &crate::Install<String>) -> anyhow::Result<Self> {
        Ok(Self(cx.params().clone()))
    }
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text(&self.0).into()
    }
}

#[derive(Default)]
struct Locked;

#[page]
impl Page for Locked {
    fn leaving(&self) -> Verdict {
        Verdict::Block
    }
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("locked").into()
    }
}

#[test]
fn navigation_callback_reuses_an_owned_route_and_preserves_history() {
    guinea_app::app::check(4, |h| {
        let mut shell = Mounted::routed::<Shell>(h, Route::Home).unwrap();
        let nav = shell.navigator::<Route>();
        let destination = Route::Details {
            name: "retained".into(),
        };
        let callback = nav.to_callback(destination.clone());

        for _ in 0..2 {
            callback.call(());
            shell.settle();
            assert_eq!(nav.current(), Some(destination.clone()));
            assert!(shell.find_text("retained").is_some());
            assert!(nav.back());
            shell.settle();
            assert_eq!(nav.current(), Some(Route::Home));
        }

        shell.click_text("Details").settle();
        assert_eq!(
            nav.current(),
            Some(Route::Details {
                name: "owned".into()
            })
        );
        assert!(shell.find_text("owned").is_some());
    });
}

#[test]
fn navigation_callback_obeys_the_current_leave_guard() {
    guinea_app::app::check(4, |h| {
        let mut shell = Mounted::routed::<Shell>(h, Route::Locked).unwrap();
        shell.click_text("Details").settle();
        assert_eq!(shell.navigator::<Route>().current(), Some(Route::Locked));
        assert!(shell.find_text("locked").is_some());
        assert!(shell.find_text("owned").is_none());
    });
}

#[test]
fn navigation_callback_does_not_keep_its_scope_alive() {
    guinea_app::app::check(1, |h| {
        let child = h.child();
        let scope = child.scope();
        let callback = Navigator::open(scope, Route::Home).to_callback(Route::Details {
            name: "late".into(),
        });
        drop(child);
        assert!(!scope.is_alive());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback.call(()))).is_err()
        );
    });
}
