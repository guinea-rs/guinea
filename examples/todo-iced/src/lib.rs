use guinea::iced::{
    Ask, Element, Envelope, Layout, LayoutCx, Page, PageCx, UpdateCx, Verdict, Window, layout, page,
};
use guinea::prelude::*;
use iced::widget::{button, column, row, scrollable, text, text_input};
use todo_core::*;

pub fn window() -> Window {
    Window::of::<Shell>()
        .title("guinea · todo")
        .at(Screen::Tasks {
            filter: Filter::All,
        })
}

#[derive(Default)]
struct Shell;

#[derive(Clone)]
struct Go(Screen);

#[layout]
impl Layout for Shell {
    type Message = Go;

    fn update(&mut self, Go(route): Go, cx: &mut UpdateCx<'_, Self>) {
        cx.navigate::<Screen>().to(route);
    }

    fn view<'a>(&'a self, cx: &mut LayoutCx<'a, Self>) -> Element<'a, Envelope> {
        let navigation = cx.mine(
            row![
                button("All").on_press(Go(Screen::Tasks {
                    filter: Filter::All
                })),
                button("Active").on_press(Go(Screen::Tasks {
                    filter: Filter::Active
                })),
                button("Completed").on_press(Go(Screen::Tasks {
                    filter: Filter::Completed
                })),
                button("Activity").on_press(Go(Screen::Activity)),
            ]
            .spacing(8),
        );
        let summary: Element<'a, Envelope> = cx.mount::<Summary>(&()).into();
        let nav = cx.navigate::<Screen>();
        let content = cx.outlet(&nav, |route, cx| match route {
            Screen::Tasks { filter } => cx.mount::<Tasks>(filter).keep(),
            Screen::Detail { id } => cx.mount::<Details>(id),
            Screen::Activity => cx.mount::<Journal>(&()),
        });
        column![navigation, summary, content]
            .spacing(12)
            .padding(16)
            .into()
    }
}

#[derive(Default)]
struct Summary;

#[derive(Clone)]
enum SummaryMsg {
    Undo,
    Clear,
    Import,
    StopImport,
    Sync,
    CancelSync,
}

#[page]
impl Page for Summary {
    type Message = SummaryMsg;

    fn update(&mut self, message: SummaryMsg, cx: &mut UpdateCx<'_, Self>) {
        let actions = cx.read::<Board>().1;
        match message {
            SummaryMsg::Undo => actions.emit(Undo),
            SummaryMsg::Clear => actions.emit(ClearCompleted),
            SummaryMsg::Import => actions.emit(Import(sample_titles())),
            SummaryMsg::StopImport => actions.emit(StopImport),
            SummaryMsg::Sync => cx.read::<SyncState>().1.emit(SyncNow),
            SummaryMsg::CancelSync => cx.read::<SyncState>().1.emit(CancelSync),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, SummaryMsg> {
        let (board, _) = cx.read::<Board>();
        let (save, _) = cx.read::<SaveState>();
        let (sync, _) = cx.read::<SyncState>();
        let (clock, _) = cx.read::<Clock>();
        column![
            text("My tasks").size(28),
            text(board.0.summary()),
            text(status(&save, &sync)),
            text(format!("Session: {} seconds", clock.0)),
            row![
                button("Undo").on_press(SummaryMsg::Undo),
                button("Clear completed").on_press(SummaryMsg::Clear),
                button("Import checklist").on_press(SummaryMsg::Import),
                button("Stop import").on_press(SummaryMsg::StopImport),
                button("Sync").on_press(SummaryMsg::Sync),
                button("Cancel sync").on_press(SummaryMsg::CancelSync),
            ]
            .spacing(6),
        ]
        .spacing(6)
        .into()
    }
}

struct Tasks {
    filter: Filter,
    title: String,
    query: String,
}

#[derive(Clone)]
enum TaskMsg {
    Title(String),
    Query(String),
    Add,
    Toggle(u64),
    Edit(u64),
    Remove(u64),
}

#[page]
impl Page for Tasks {
    type Params = Filter;
    type Message = TaskMsg;

    fn install(cx: &Install<Filter>) -> anyhow::Result<Self> {
        cx.install::<Search>(&())?;
        Ok(Self {
            filter: *cx.params(),
            title: String::new(),
            query: String::new(),
        })
    }

    fn update(&mut self, message: TaskMsg, cx: &mut UpdateCx<'_, Self>) {
        let actions = cx.read::<Board>().1;
        match message {
            TaskMsg::Title(title) => self.title = title,
            TaskMsg::Query(query) => {
                self.query = query;
                cx.read::<SearchResults>().1.emit(Query(self.query.clone()));
            }
            TaskMsg::Add => actions.emit(Add(std::mem::take(&mut self.title))),
            TaskMsg::Toggle(id) => actions.emit(Toggle(id)),
            TaskMsg::Edit(id) => {
                cx.navigate::<Screen>().to(Screen::Detail { id });
            }
            TaskMsg::Remove(id) => actions.emit(Remove(id)),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, TaskMsg> {
        let (board, _) = cx.read::<Board>();
        let (search, _) = cx.read::<SearchResults>();
        let mut rows = column![
            row![
                text_input("New task", &self.title)
                    .on_input(TaskMsg::Title)
                    .on_submit(TaskMsg::Add),
                button("Add task").on_press(TaskMsg::Add)
            ]
            .spacing(8),
            text_input("Search tasks", &self.query).on_input(TaskMsg::Query),
        ]
        .spacing(8);
        if search.pending {
            rows = rows.push(text("Searching…"));
        }
        for todo in visible(&board.0, &search, self.filter) {
            rows = rows.push(
                row![
                    button(text(format!(
                        "{} {}",
                        if todo.done { "[x]" } else { "[ ]" },
                        todo.title
                    )))
                    .on_press(TaskMsg::Toggle(todo.id)),
                    button("Edit").on_press(TaskMsg::Edit(todo.id)),
                    button("Delete").on_press(TaskMsg::Remove(todo.id)),
                ]
                .spacing(8),
            );
        }
        scrollable(rows).into()
    }
}

struct Details {
    id: u64,
    title: String,
    saved: String,
}

#[derive(Clone)]
enum DetailMsg {
    Title(String),
    Save,
    Back,
}

#[page]
impl Page for Details {
    type Params = u64;
    type Message = DetailMsg;

    fn install(cx: &Install<u64>) -> anyhow::Result<Self> {
        let board = cx
            .read::<Board>()
            .ok_or_else(|| anyhow::anyhow!("no board"))?;
        let todo = board
            .0
            .todos
            .iter()
            .find(|todo| todo.id == *cx.params())
            .ok_or_else(|| anyhow::anyhow!("task was removed"))?;
        Ok(Self {
            id: todo.id,
            title: todo.title.clone(),
            saved: todo.title.clone(),
        })
    }

    fn leaving(&self) -> Verdict {
        if self.title == self.saved {
            Verdict::Allow
        } else {
            Verdict::ask(Ask::new(
                "Discard the unsaved title?",
                "Discard",
                "Keep editing",
            ))
        }
    }

    fn update(&mut self, message: DetailMsg, cx: &mut UpdateCx<'_, Self>) {
        match message {
            DetailMsg::Title(title) => self.title = title,
            DetailMsg::Save if !self.title.trim().is_empty() => {
                cx.read::<Board>()
                    .1
                    .emit(Rename(self.id, self.title.clone()));
                self.saved = self.title.clone();
                cx.navigate::<Screen>().back();
            }
            DetailMsg::Save => {}
            DetailMsg::Back => {
                cx.navigate::<Screen>().back();
            }
        }
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, DetailMsg> {
        column![
            text(format!("Task {}", self.id)),
            text_input("Title", &self.title)
                .on_input(DetailMsg::Title)
                .on_submit(DetailMsg::Save),
            row![
                button("Save title").on_press(DetailMsg::Save),
                button("Back").on_press(DetailMsg::Back)
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into()
    }
}

#[derive(Default)]
struct Journal;

#[page]
impl Page for Journal {
    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Self::Message> {
        let (activity, _) = cx.read::<Activity>();
        let mut rows = column![text("Activity log").size(24)].spacing(6);
        for entry in &activity.0 {
            rows = rows.push(text(entry.clone()));
        }
        scrollable(rows).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_a_title_requires_confirmation_before_leaving() {
        let mut page = Details {
            id: 1,
            title: "saved".into(),
            saved: "saved".into(),
        };
        assert!(matches!(page.leaving(), Verdict::Allow));
        page.title = "edited".into();
        assert!(matches!(page.leaving(), Verdict::Ask(..)));
    }
}
