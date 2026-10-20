use guinea::prelude::*;
use guinea::winui::reactor::{
    Button, IntoUnitCallback, Orientation, StackPanel, TextBlock, TextBox, View,
};
use guinea::winui::{Ask, Layout, LayoutCx, Page, PageCx, UpdateCx, Verdict, Window, layout, page};
use std::rc::Rc;
use todo_core::*;

pub fn window() -> Window {
    Window::of::<Shell>()
        .title("guinea · todo")
        .at(Screen::Tasks {
            filter: Filter::All,
        })
}

fn button(label: impl Into<String>, clicked: impl IntoUnitCallback) -> View {
    Button::new()
        .content(TextBlock::new().text(label.into()))
        .on_click(clicked)
        .into()
}

fn row(children: Vec<View>) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children(children)
        .into()
}

#[guinea::slot]
struct EditorTools;

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Screen>();
        let mut navigation = Vec::new();
        for (label, filter) in [
            ("All", Filter::All),
            ("Active", Filter::Active),
            ("Completed", Filter::Completed),
        ] {
            navigation.push(button(label, nav.to_handler(Screen::Tasks { filter })));
        }
        navigation.push(button("Activity", nav.to_handler(Screen::Activity)));
        let summary = cx.mount::<Summary>(&()).into();
        let tools = cx.slot::<EditorTools>();
        let content = cx.outlet(&nav, |route, cx| match route {
            Screen::Tasks { filter } => cx.mount::<Tasks>(filter).keep(),
            Screen::Detail { id } => cx.mount::<Details>(id),
            Screen::Activity => cx.mount::<Journal>(&()),
        });
        StackPanel::new()
            .spacing(12.0)
            .children(vec![row(navigation), summary, tools, content])
            .into()
    }
}

#[derive(Default)]
struct Summary;

#[page]
impl Page for Summary {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (board, actions) = cx.read::<Board>();
        let (save, _) = cx.read::<SaveState>();
        let (sync, syncing) = cx.read::<SyncState>();
        let (clock, _) = cx.read::<Clock>();
        StackPanel::new()
            .spacing(8.0)
            .children(vec![
                TextBlock::new().text("My tasks").into(),
                TextBlock::new().text(board.0.summary()).into(),
                TextBlock::new().text(status(&save, &sync)).into(),
                TextBlock::new()
                    .text(format!("Session: {} seconds", clock.0))
                    .into(),
                row(vec![
                    button("Undo", move || actions.emit(Undo)),
                    button("Clear completed", move || actions.emit(ClearCompleted)),
                    button("Import checklist", move || {
                        actions.emit(Import(sample_titles()))
                    }),
                    button("Stop import", move || actions.emit(StopImport)),
                    button("Sync", move || syncing.emit(SyncNow)),
                    button("Cancel sync", move || syncing.emit(CancelSync)),
                ]),
            ])
            .into()
    }
}

struct Tasks {
    filter: Filter,
    title: String,
    query: String,
}

enum TaskMsg {
    Title(String),
    Query(String),
    Add,
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
        match message {
            TaskMsg::Title(title) => self.title = title,
            TaskMsg::Query(query) => {
                self.query = query;
                cx.read::<SearchResults>().1.emit(Query(self.query.clone()));
            }
            TaskMsg::Add => {
                cx.read::<Board>()
                    .1
                    .emit(Add(std::mem::take(&mut self.title)));
            }
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (board, actions) = cx.read::<Board>();
        let (search, _) = cx.read::<SearchResults>();
        let nav = cx.navigate::<Screen>();
        let mut children = vec![
            TextBlock::new().text("New task").into(),
            TextBox::new(&self.title)
                .on_text_changed(cx.on(|text: Rc<str>| TaskMsg::Title(text.to_string())))
                .into(),
            button("Add task", cx.on(|()| TaskMsg::Add)),
            TextBlock::new().text("Search").into(),
            TextBox::new(&self.query)
                .on_text_changed(cx.on(|text: Rc<str>| TaskMsg::Query(text.to_string())))
                .into(),
        ];
        if search.pending {
            children.push(TextBlock::new().text("Searching…").into());
        }
        for todo in visible(&board.0, &search, self.filter) {
            let id = todo.id;
            children.push(row(vec![
                button(
                    format!("{} {}", if todo.done { "[x]" } else { "[ ]" }, todo.title),
                    move || actions.emit(Toggle(id)),
                ),
                button(format!("Edit {id}"), nav.to_handler(Screen::Detail { id })),
                button(format!("Delete {id}"), move || actions.emit(Remove(id))),
            ]));
        }
        StackPanel::new().spacing(8.0).children(children).into()
    }
}

struct Details {
    id: u64,
    title: String,
    saved: String,
}

enum DetailMsg {
    Title(String),
    Save(guinea::winui::Navigator<Screen>),
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
            DetailMsg::Save(nav) => {
                if !self.title.trim().is_empty() {
                    cx.read::<Board>()
                        .1
                        .emit(Rename(self.id, self.title.clone()));
                    self.saved = self.title.clone();
                    nav.back();
                }
            }
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Screen>();
        let saving = nav.clone();
        cx.fill::<EditorTools>(button(
            "Save title",
            cx.on(move |()| DetailMsg::Save(saving.clone())),
        ));
        StackPanel::new()
            .spacing(8.0)
            .children(vec![
                TextBlock::new().text(format!("Task {}", self.id)).into(),
                TextBox::new(&self.title)
                    .on_text_changed(cx.on(|text: Rc<str>| DetailMsg::Title(text.to_string())))
                    .into(),
                button("Back", move || {
                    nav.back();
                }),
            ])
            .into()
    }
}

#[derive(Default)]
struct Journal;

#[page]
impl Page for Journal {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (activity, _) = cx.read::<Activity>();
        let mut children = vec![TextBlock::new().text("Activity log").into()];
        children.extend(
            activity
                .0
                .iter()
                .map(|entry| TextBlock::new().text(entry.clone()).into()),
        );
        StackPanel::new().spacing(6.0).children(children).into()
    }
}
