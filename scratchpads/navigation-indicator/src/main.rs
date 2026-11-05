use std::{rc::Rc, time::Duration};
use windows_reactor::*;

#[derive(Clone, PartialEq)]
struct Options {
    controlled: bool,
    icons: bool,
    static_content: bool,
    tick: bool,
}
impl Options {
    fn read() -> Self {
        let args: Vec<_> = std::env::args().collect();
        Self {
            controlled: !args.iter().any(|a| a == "--uncontrolled"),
            icons: !args.iter().any(|a| a == "--no-icons"),
            static_content: args.iter().any(|a| a == "--static-content"),
            tick: args.iter().any(|a| a == "--tick"),
        }
    }
}
const ITEMS: [(&str, &str); 4] = [
    ("Processes", "\u{ECAA}"),
    ("Activity", "\u{E9D9}"),
    ("Services", "\u{E81C}"),
    ("Settings", "\u{E713}"),
];
fn selected(tag: Option<Rc<str>>) -> Option<usize> {
    tag.and_then(|tag| ITEMS.iter().position(|(name, _)| *name == tag.as_ref()))
}
fn navigation(
    options: &Options,
    current: usize,
    redraws: u64,
    open: bool,
    content: View,
    on_select: Callback<usize>,
    on_redraw: Callback<()>,
    on_next: Callback<()>,
    on_pane: Callback<bool>,
) -> View {
    let item = |index: usize| {
        let (name, glyph) = ITEMS[index];
        let mut item = NavigationViewItem::new().tag(name).content(name);
        if options.controlled {
            item = item.is_selected(index == current);
        }
        if options.icons {
            item = item.icon(Icon::font(glyph));
        }
        keyed(name, item)
    };
    NavigationView::new()
        .keyed_menu_items((0..3).map(item).collect::<Vec<_>>())
        .keyed_footer_menu_items(vec![item(3)])
        .is_settings_visible(false)
        .pane_display_mode(NavigationViewPaneDisplayMode::Left)
        .is_pane_open(open)
        .open_pane_length(240.0)
        .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
        .on_is_pane_open_changed(on_pane)
        .on_selected_tag_changed(move |tag| {
            if let Some(index) = selected(tag) {
                on_select.call(index)
            }
        })
        .pane_footer(
            StackPanel::new().spacing(8.0).children((
                TextBlock::new().text(format!("Redraws: {redraws}")),
                Button::new()
                    .content("Redraw same selection")
                    .on_click(on_redraw),
                Button::new()
                    .content("Next (programmatic)")
                    .is_enabled(options.controlled)
                    .on_click(on_next),
            )),
        )
        .content(content)
        .into()
}
fn content(index: usize) -> View {
    Border::new()
        .padding(Thickness::uniform(24.0))
        .content(TextBlock::new().text(ITEMS[index].0).font_size(28.0))
        .into()
}
#[derive(Clone, Copy)]
enum Msg {
    Select(usize),
    Redraw,
    Next,
    Pane(bool),
}
struct Raw {
    selected: usize,
    redraws: u64,
    open: bool,
    timer: Option<ComponentTimer>,
}
impl Component for Raw {
    type Input = Options;
    type Message = Msg;
    fn create(options: &Options, cx: &ComponentContext<Self>) -> Self {
        Self {
            selected: 0,
            redraws: 0,
            open: true,
            timer: options
                .tick
                .then(|| cx.set_local_timeout(Duration::from_secs(1), || Msg::Redraw)),
        }
    }
    fn update(&mut self, msg: Msg, cx: &ComponentContext<Self>) {
        match msg {
            Msg::Select(i) => {
                eprintln!("Selected {} -> {}", self.selected, i);
                self.selected = i;
            }
            Msg::Next => self.selected = (self.selected + 1) % 4,
            Msg::Pane(v) => self.open = v,
            Msg::Redraw => self.redraws += 1,
        }
        if self.timer.is_some() {
            self.timer = Some(cx.set_local_timeout(Duration::from_secs(1), || Msg::Redraw));
        }
    }
    fn view(&self, options: &Options, cx: &mut ViewContext<Self>) -> View {
        cx.window_title("Navigation indicator scratchpad - Reactor");
        navigation(
            options,
            self.selected,
            self.redraws,
            self.open,
            content(if options.static_content {
                0
            } else {
                self.selected
            }),
            cx.callback(Msg::Select),
            cx.message(Msg::Redraw),
            cx.message(Msg::Next),
            cx.callback(Msg::Pane),
        )
    }
}
mod integrated {
    use super::*;
    use guinea::winui::{Layout, LayoutCx, Page, PageCx, UpdateCx, Window, layout, mount, page};
    #[derive(Clone, Debug, PartialEq, guinea::Routable)]
    enum Route {
        Processes,
        Activity,
        Services,
        Settings,
    }
    fn route(i: usize) -> Route {
        [
            Route::Processes,
            Route::Activity,
            Route::Services,
            Route::Settings,
        ][i]
            .clone()
    }
    fn index(route: Option<&Route>) -> usize {
        match route {
            Some(Route::Processes) => 0,
            Some(Route::Activity) => 1,
            Some(Route::Services) => 2,
            _ => 3,
        }
    }
    #[derive(Default)]
    struct Shell {
        redraws: u64,
        closed: bool,
    }
    enum LayoutMsg {
        Redraw,
        Pane(bool),
    }
    #[layout]
    impl Layout for Shell {
        type Message = LayoutMsg;
        fn update(&mut self, msg: LayoutMsg, _: &mut UpdateCx<'_, Self>) {
            match msg {
                LayoutMsg::Redraw => self.redraws += 1,
                LayoutMsg::Pane(open) => self.closed = !open,
            }
        }
        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let options = Options::read();
            let nav = cx.navigate::<Route>();
            let current = index(nav.current().as_ref());
            let page: View = if options.static_content {
                content(0)
            } else {
                cx.outlet::<Route>(|route| match route {
                    Route::Processes => mount::<Processes>(()),
                    Route::Activity => mount::<Activity>(()),
                    Route::Services => mount::<Services>(()),
                    Route::Settings => mount::<Settings>(()),
                })
                .into()
            };
            navigation(
                &options,
                current,
                self.redraws,
                !self.closed,
                page,
                Callback::new(move |i| {
                    let _ = nav.to(route(i));
                }),
                cx.on(|()| LayoutMsg::Redraw),
                Callback::new(move |()| {
                    let _ = nav.to(route((current + 1) % 4));
                }),
                cx.on(LayoutMsg::Pane),
            )
        }
    }
    macro_rules! leaf {
        ($name:ident, $i:expr) => {
            #[derive(Default)]
            struct $name;
            #[page]
            impl Page for $name {
                fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
                    content($i)
                }
            }
        };
    }
    leaf!(Processes, 0);
    leaf!(Activity, 1);
    leaf!(Services, 2);
    leaf!(Settings, 3);
    pub fn run() -> anyhow::Result<()> {
        guinea::winui::run(|_| {
            Ok(Window::of::<Shell>()
                .title("Navigation indicator scratchpad - Guinea")
                .at(Route::Processes))
        })
    }
}
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--guinea") {
        anyhow::ensure!(
            !Options::read().tick,
            "--tick is only supported by the raw Reactor mode"
        );
        integrated::run()
    } else {
        App::run_component::<Raw>(Options::read())?;
        Ok(())
    }
}
