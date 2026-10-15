//! Frames without a terminal: ratatui draws into a test backend nobody
//! shows, which is all a mount, a key and a navigation need.

use std::cell::RefCell;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::guard::{Ask, Verdict};
use guinea_core::scope::Scope;
use guinea_macros::{feature, installs, reducer};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout as Rows};
use ratatui::widgets::Paragraph;
use ratatui::{Terminal, backend::TestBackend};

use crate::run::{Root, frame, hear, open};
use crate::{Handled, InputCx, Layout, LayoutCx, Navigator, Page, PageCx, Routable, closed, layout, offer, page};

#[derive(Clone, PartialEq, Debug, Routable)]
enum Place {
    Processes,
    Remembering,
    Other,
    Broken,
    Reading,
    Draft,
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Shared(u32);

#[reducer]
fn shared(this: &mut Shared, to: u32) {
    this.0 = to;
}

feature! {
    Sharing {
        exports { Shared }
    }
}

#[installs]
fn sharing(cx: &FeatureInitContext) -> anyhow::Result<Sharing> {
    Ok(Sharing(cx.state::<Shared>().seed(Shared(7)).plain()))
}

/// Chrome on the first row, the page under it. Takes `x` when nothing below
/// did, `o` to go elsewhere, `q` to close.
#[derive(Default)]
struct Shell {
    took: bool,
}

#[layout]
impl Layout for Shell {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Sharing>(&())?;
        Ok(Self::default())
    }

    fn render(&mut self, cx: &mut LayoutCx<'_, '_, Self>) {
        let rows = Rows::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(cx.area());

        let chrome = if self.took { "shell took x" } else { "tabs" };
        cx.frame().render_widget(Paragraph::new(chrome), rows[0]);

        let nav = cx.navigate::<Place>();
        let page = cx.outlet(&nav, |place, cx| match place {
            Place::Processes => cx.mount::<Processes>(&()),
            Place::Remembering => cx.mount::<Remembering>(&()).keep(),
            Place::Other => cx.mount::<Other>(&()),
            Place::Broken => cx.mount::<Broken>(&()),
            Place::Reading => cx.mount::<Reading>(&()),
            Place::Draft => cx.mount::<Draft>(&()),
        });
        page.draw(cx.frame(), rows[1]);
    }

    fn on_key(&mut self, cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        match key.code {
            KeyCode::Char('x') => self.took = true,
            KeyCode::Char('o') => {
                let _moved = cx.navigate::<Place>().to(Place::Other);
            }
            KeyCode::Char('q') => cx.close(),
            _ => return Handled::No,
        }
        Handled::Yes
    }
}

/// Counts its frames, and keeps what is typed or pasted into it.
#[derive(Default)]
struct Processes {
    frames: u32,
    filter: String,
}

#[page]
impl Page for Processes {
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        self.frames += 1;

        let area = cx.area();
        let shown = format!("processes {} {}", self.frames, self.filter);
        cx.frame().render_widget(Paragraph::new(shown.trim_end().to_string()), area);
    }

    fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        match key.code {
            KeyCode::Char(typed @ ('x' | 'y')) => {
                self.filter.push(typed);
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn on_paste(&mut self, _cx: &mut InputCx<'_, Self>, text: &str) -> Handled {
        self.filter.push_str(text);
        Handled::Yes
    }
}

/// [`Processes`]' counting, mounted to be kept.
#[derive(Default)]
struct Remembering {
    frames: u32,
}

#[page]
impl Page for Remembering {
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        self.frames += 1;

        let area = cx.area();
        let shown = format!("remembering {}", self.frames);
        cx.frame().render_widget(Paragraph::new(shown), area);
    }
}

/// Takes every `x`, so a key it was offered is visible as one it ate.
#[derive(Default)]
struct Other {
    took: u32,
}

#[page]
impl Page for Other {
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let area = cx.area();
        let shown = format!("other {}", self.took);
        cx.frame().render_widget(Paragraph::new(shown), area);
    }

    fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        if key.code != KeyCode::Char('x') {
            return Handled::No;
        }
        self.took += 1;
        Handled::Yes
    }
}

#[derive(Default)]
struct Broken;

#[page]
impl Page for Broken {
    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        anyhow::bail!("the agent is not reachable")
    }

    fn render(&mut self, _cx: &mut PageCx<'_, '_, Self>) {}
}

#[derive(Default)]
struct Reading;

#[page]
impl Page for Reading {
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let (shared, _) = cx.read::<Shared>();

        let area = cx.area();
        cx.frame().render_widget(Paragraph::new(format!("read {}", shared.0)), area);
    }
}

/// Minds being left: every attempt to go puts a question. Takes `x` as typing.
#[derive(Default)]
struct Draft {
    typed: u32,
}

#[page]
impl Page for Draft {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.on_leave(|| Verdict::ask(Ask::new("Discard the draft?", "Discard", "Keep editing")));
        Ok(Self::default())
    }

    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let area = cx.area();
        let shown = format!("draft {}", self.typed);
        cx.frame().render_widget(Paragraph::new(shown), area);
    }

    fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        if key.code != KeyCode::Char('x') {
            return Handled::No;
        }
        self.typed += 1;
        Handled::Yes
    }
}

struct Window {
    host: FeatureHost,
    nav: Navigator<Place>,
    terminal: RefCell<Terminal<TestBackend>>,
}

impl Window {
    fn at(place: Place) -> Self {
        let host = FeatureHost::detached();
        open(host.scope());
        let nav = Navigator::open(host.scope(), place);
        let window = Self {
            host,
            nav,
            terminal: RefCell::new(Terminal::new(TestBackend::new(20, 2)).expect("terminal")),
        };
        window.draw();
        window
    }

    fn scope(&self) -> Scope {
        self.host.scope()
    }

    fn draw(&self) -> String {
        let root = Root::of::<Shell>();
        let window = self.scope();
        let mut terminal = self.terminal.borrow_mut();
        terminal
            .draw(|drawing| {
                let area = drawing.area();
                frame(window, &root, drawing, area)
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn offer(&self, event: Event) -> Handled {
        offer(self.scope(), &event)
    }

    fn hear(&self, event: Event) {
        hear(self.scope(), Some(&self.nav), &event);
    }

    fn go(&self, place: Place) -> String {
        let _moved = self.nav.to(place);
        self.draw()
    }
}

fn press(typed: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(typed), KeyModifiers::NONE))
}

fn release(typed: char) -> Event {
    Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char(typed),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ))
}

#[test]
fn what_is_mounted_says_where_it_was_written() {
    let window = Window::at(Place::Processes);
    let root = guinea_app::app::roots::of(window.scope()).map(|root| root.get());

    let written: Option<Vec<&str>> = root.and_then(guinea_router::observability::window).map(|view| {
        view.mounted
            .iter()
            .filter(|mounted| mounted.written.is_some_and(|at| at.file.ends_with("tests.rs")))
            .map(|mounted| mounted.name)
            .collect()
    });

    assert_eq!(written, Some(vec!["Shell", "Processes"]));
}

#[test]
fn a_layout_places_the_page_it_mounts() {
    let window = Window::at(Place::Processes);

    assert_eq!(window.draw(), "tabs\nprocesses 2");
}

#[test]
fn a_page_keeps_its_own_state_between_frames() {
    let window = Window::at(Place::Processes);
    window.draw();

    assert_eq!(window.draw(), "tabs\nprocesses 3");
}

#[test]
fn a_key_is_offered_to_the_page_before_the_layout() {
    let window = Window::at(Place::Processes);

    assert_eq!(window.offer(press('x')), Handled::Yes);
    assert_eq!(window.draw(), "tabs\nprocesses 2 x");
}

#[test]
fn a_key_the_page_leaves_reaches_the_layout() {
    let window = Window::at(Place::Reading);

    assert_eq!(window.offer(press('x')), Handled::Yes);
    assert_eq!(window.draw(), "shell took x\nread 7");
}

#[test]
fn a_key_nobody_takes_is_left_alone() {
    let window = Window::at(Place::Processes);

    assert_eq!(window.offer(press('z')), Handled::No);
    assert_eq!(window.draw(), "tabs\nprocesses 2");
}

#[test]
fn a_release_is_not_offered() {
    let window = Window::at(Place::Processes);

    assert_eq!(window.offer(release('x')), Handled::No);
    assert_eq!(window.draw(), "tabs\nprocesses 2");
}

#[test]
fn a_paste_is_offered_as_one_text() {
    let window = Window::at(Place::Processes);

    assert_eq!(window.offer(Event::Paste("db-01".into())), Handled::Yes);
    assert_eq!(window.draw(), "tabs\nprocesses 2 db-01");
}

#[test]
fn a_move_made_by_a_key_is_drawn_in_the_next_frame() {
    let window = Window::at(Place::Processes);

    assert_eq!(window.offer(press('o')), Handled::Yes);

    assert_eq!(window.draw(), "tabs\nother 0");
}

#[test]
fn a_page_that_was_left_is_not_offered_keys() {
    let window = Window::at(Place::Remembering);
    window.go(Place::Other);

    assert_eq!(window.offer(press('x')), Handled::Yes);

    assert_eq!(window.draw(), "tabs\nother 1");
}

#[test]
fn a_kept_page_comes_back_as_it_was_left() {
    let window = Window::at(Place::Remembering);
    window.draw();

    window.go(Place::Other);

    assert_eq!(window.go(Place::Remembering), "tabs\nremembering 3");
}

#[test]
fn a_page_not_kept_starts_again() {
    let window = Window::at(Place::Processes);
    window.draw();

    window.go(Place::Other);

    assert_eq!(window.go(Place::Processes), "tabs\nprocesses 1");
}

#[test]
fn a_page_reads_what_the_layout_above_exports() {
    let window = Window::at(Place::Reading);

    assert_eq!(window.draw(), "tabs\nread 7");
}

#[test]
fn a_place_that_fails_to_mount_leaves_the_window_where_it_was() {
    let window = Window::at(Place::Other);

    assert_eq!(window.go(Place::Broken), "tabs\nother 0");
    assert_eq!(window.nav.current(), Some(Place::Other));
    assert_eq!(
        window.nav.failure().as_deref(),
        Some("Broken did not mount: the agent is not reachable")
    );
}

#[test]
fn a_segment_closes_the_window() {
    let window = Window::at(Place::Processes);
    assert!(!closed(window.scope()));

    window.offer(press('q'));

    assert!(closed(window.scope()));
}

#[test]
fn while_a_question_is_up_keys_answer_it_and_reach_nothing_else() {
    let window = Window::at(Place::Draft);
    window.go(Place::Other);
    assert!(window.nav.pending().is_some(), "the draft minds being left");

    window.hear(press('x'));
    assert_eq!(window.draw(), "tabs\ndraft 0", "the key went to the question");

    window.hear(press('y'));
    assert_eq!(window.draw(), "tabs\nother 0");
}

#[test]
fn without_a_question_keys_reach_what_was_drawn() {
    let window = Window::at(Place::Draft);

    window.hear(press('x'));

    assert_eq!(window.draw(), "tabs\ndraft 1");
}
