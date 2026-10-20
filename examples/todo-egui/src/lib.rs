use guinea::eframe::{Layout, LayoutCx, Page, PageCx, Window, layout, page};
use guinea::prelude::*;
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

#[layout]
impl Layout for Shell {
    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<Screen>();
        cx.ui().horizontal(|ui| {
            for (label, filter) in [
                ("All", Filter::All),
                ("Active", Filter::Active),
                ("Completed", Filter::Completed),
            ] {
                if ui.button(label).clicked() {
                    nav.to(Screen::Tasks { filter });
                }
            }
            if ui.button("Activity").clicked() {
                nav.to(Screen::Activity);
            }
        });
        let summary = cx.mount::<Summary>(&());
        let content = cx.outlet(&nav, |route, cx| match route {
            Screen::Tasks { filter } => cx.mount::<Tasks>(filter).keep(),
            Screen::Detail { id } => cx.mount::<Details>(id),
            Screen::Activity => cx.mount::<Journal>(&()),
        });
        summary.draw(cx.ui());
        content.draw(cx.ui());
    }
}

#[derive(Default)]
struct Summary;

#[page]
impl Page for Summary {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (board, actions) = cx.read::<Board>();
        let (save, _) = cx.read::<SaveState>();
        let (sync, syncing) = cx.read::<SyncState>();
        let (clock, _) = cx.read::<Clock>();
        let ui = cx.ui();
        ui.heading("My tasks");
        ui.label(board.0.summary());
        ui.label(status(&save, &sync));
        ui.label(format!("Session: {} seconds", clock.0));
        ui.horizontal(|ui| {
            if ui.button("Undo").clicked() {
                actions.emit(Undo);
            }
            if ui.button("Clear completed").clicked() {
                actions.emit(ClearCompleted);
            }
            if ui.button("Import checklist").clicked() {
                actions.emit(Import(sample_titles()));
            }
            if ui.button("Stop import").clicked() {
                actions.emit(StopImport);
            }
            if ui.button("Sync").clicked() {
                syncing.emit(SyncNow);
            }
            if ui.button("Cancel sync").clicked() {
                syncing.emit(CancelSync);
            }
        });
        ui.separator();
    }
}

struct Tasks {
    filter: Filter,
    title: String,
    query: String,
}

#[page]
impl Page for Tasks {
    type Params = Filter;

    fn install(cx: &Install<Filter>) -> anyhow::Result<Self> {
        cx.install::<Search>(&())?;
        Ok(Self {
            filter: *cx.params(),
            title: String::new(),
            query: String::new(),
        })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (board, actions) = cx.read::<Board>();
        let (search, searching) = cx.read::<SearchResults>();
        let nav = cx.navigate::<Screen>();
        let ui = cx.ui();
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.title);
            if ui.button("Add task").clicked() && !self.title.trim().is_empty() {
                actions.emit(Add(std::mem::take(&mut self.title)));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Search:");
            if ui.text_edit_singleline(&mut self.query).changed() {
                searching.emit(Query(self.query.clone()));
            }
            if search.pending {
                ui.label("Searching…");
            }
        });
        for todo in visible(&board.0, &search, self.filter) {
            ui.horizontal(|ui| {
                let mut done = todo.done;
                if ui.checkbox(&mut done, &todo.title).changed() {
                    actions.emit(Toggle(todo.id));
                }
                if ui.button(format!("Edit {}", todo.id)).clicked() {
                    nav.to(Screen::Detail { id: todo.id });
                }
                if ui.button(format!("Delete {}", todo.id)).clicked() {
                    actions.emit(Remove(todo.id));
                }
            });
        }
    }
}

struct Details {
    id: u64,
    title: String,
}

#[page]
impl Page for Details {
    type Params = u64;

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
        })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (_, actions) = cx.read::<Board>();
        let nav = cx.navigate::<Screen>();
        let ui = cx.ui();
        ui.heading(format!("Task {}", self.id));
        ui.text_edit_singleline(&mut self.title);
        if ui.button("Save title").clicked() {
            actions.emit(Rename(self.id, self.title.clone()));
            nav.back();
        }
        if ui.button("Back").clicked() {
            nav.back();
        }
    }
}

#[derive(Default)]
struct Journal;

#[page]
impl Page for Journal {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (activity, _) = cx.read::<Activity>();
        cx.ui().heading("Activity log");
        for entry in &activity.0 {
            cx.ui().label(entry);
        }
    }
}
