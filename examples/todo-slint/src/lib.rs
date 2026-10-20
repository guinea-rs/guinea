use std::rc::Rc;

use guinea::prelude::*;
use guinea::slint::{BindCx, Layout, LayoutCx, Navigator, Page, Shows, Window, layout, page};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use todo_core::*;

slint::slint! {
    import { Button, CheckBox, LineEdit, ListView, VerticalBox, HorizontalBox } from "std-widgets.slint";

    export struct TodoRow { id: string, title: string, completed: bool }

    export global TodoUi {
        in-out property <int> screen;
        in-out property <string> summary;
        in-out property <string> status;
        in-out property <string> clock;
        in-out property <string> title;
        in-out property <string> query;
        in-out property <string> detail-title;
        in-out property <[TodoRow]> todos;
        in-out property <[string]> entries;
        callback go(int);
        callback command(int);
        callback add(string);
        callback search(string);
        callback toggle(string);
        callback remove(string);
        callback edit(string);
        callback rename(string);
        callback back();
    }

    export component AppWindow inherits Window {
        title: "guinea · todo";
        width: 760px;
        height: 600px;
        VerticalBox {
            HorizontalBox {
                Button { text: "All"; clicked => { TodoUi.go(0); } }
                Button { text: "Active"; clicked => { TodoUi.go(1); } }
                Button { text: "Completed"; clicked => { TodoUi.go(2); } }
                Button { text: "Activity"; clicked => { TodoUi.go(3); } }
            }
            Text { text: "My tasks"; font-size: 24px; }
            Text { text: TodoUi.summary; }
            Text { text: TodoUi.status; }
            Text { text: TodoUi.clock; }
            HorizontalBox {
                Button { text: "Undo"; clicked => { TodoUi.command(0); } }
                Button { text: "Clear completed"; clicked => { TodoUi.command(1); } }
                Button { text: "Import checklist"; clicked => { TodoUi.command(2); } }
                Button { text: "Stop import"; clicked => { TodoUi.command(3); } }
                Button { text: "Sync"; clicked => { TodoUi.command(4); } }
                Button { text: "Cancel sync"; clicked => { TodoUi.command(5); } }
            }
            if TodoUi.screen < 3: VerticalBox {
                HorizontalBox {
                    LineEdit { text <=> TodoUi.title; placeholder-text: "New task"; accepted => { TodoUi.add(self.text); } }
                    Button { text: "Add task"; clicked => { TodoUi.add(TodoUi.title); } }
                }
                LineEdit { text <=> TodoUi.query; placeholder-text: "Search tasks"; edited => { TodoUi.search(self.text); } }
                ListView {
                    for todo in TodoUi.todos: HorizontalBox {
                        visible: TodoUi.screen == 0 || (TodoUi.screen == 1 && !todo.completed) || (TodoUi.screen == 2 && todo.completed);
                        CheckBox { text: todo.title; checked: todo.completed; toggled => { TodoUi.toggle(todo.id); } }
                        Button { text: "Edit"; clicked => { TodoUi.edit(todo.id); } }
                        Button { text: "Delete"; clicked => { TodoUi.remove(todo.id); } }
                    }
                }
            }
            if TodoUi.screen == 3: ListView {
                for entry in TodoUi.entries: Text { text: entry; }
            }
            if TodoUi.screen == 4: VerticalBox {
                Text { text: "Edit task"; }
                LineEdit { text <=> TodoUi.detail-title; }
                HorizontalBox {
                    Button { text: "Save title"; clicked => { TodoUi.rename(TodoUi.detail-title); } }
                    Button { text: "Back"; clicked => { TodoUi.back(); } }
                }
            }
        }
    }
}

impl Shows<AppWindow> for Screen {
    fn show(window: &AppWindow, nav: &Navigator<Self>) {
        let screen = match nav.current() {
            Some(Screen::Tasks {
                filter: Filter::All,
            }) => 0,
            Some(Screen::Tasks {
                filter: Filter::Active,
            }) => 1,
            Some(Screen::Tasks {
                filter: Filter::Completed,
            }) => 2,
            Some(Screen::Activity) => 3,
            _ => 4,
        };
        window.global::<TodoUi>().set_screen(screen);
    }

    fn listen(window: &AppWindow, nav: &Navigator<Self>) {
        let navigating = nav.clone();
        window.global::<TodoUi>().on_go(move |index| {
            let route = match index {
                0 => Screen::Tasks {
                    filter: Filter::All,
                },
                1 => Screen::Tasks {
                    filter: Filter::Active,
                },
                2 => Screen::Tasks {
                    filter: Filter::Completed,
                },
                _ => Screen::Activity,
            };
            navigating.to(route);
        });
        let navigating = nav.clone();
        window.global::<TodoUi>().on_back(move || {
            navigating.back();
        });
    }
}

pub fn window() -> anyhow::Result<Window<AppWindow>> {
    Ok(window_for(AppWindow::new()?))
}

pub fn window_for(component: AppWindow) -> Window<AppWindow> {
    Window::of::<Shell>(component).at(Screen::Tasks {
        filter: Filter::All,
    })
}

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn mount(&mut self, cx: &mut LayoutCx<'_, Self>) {
        cx.mount::<Summary>(&());
        let nav = cx.navigate::<Screen>();
        cx.outlet(&nav, |route, cx| match route {
            Screen::Tasks { .. } => cx.mount::<Tasks>(&()).keep(),
            Screen::Detail { id } => cx.mount::<Details>(id),
            Screen::Activity => cx.mount::<Journal>(&()),
        });
    }
}

#[derive(Default)]
struct Summary;

#[page]
impl Page for Summary {
    fn bind(&mut self, cx: &BindCx<Self>) {
        let root = cx.root::<AppWindow>();
        cx.bind_to::<Board, _>(&root, |root, board| {
            root.global::<TodoUi>()
                .set_summary(board.0.summary().into())
        });
        cx.bind_to::<Clock, _>(&root, |root, clock| {
            root.global::<TodoUi>()
                .set_clock(format!("Session: {} seconds", clock.0).into())
        });
        let save = cx.binding::<SaveState>();
        let sync = cx.binding::<SyncState>();
        let refreshing = root.clone_strong();
        let refresh = Rc::new(move || {
            refreshing
                .global::<TodoUi>()
                .set_status(status(&save.get(), &sync.get()).into())
        });
        let refreshed = refresh.clone();
        cx.bind::<SaveState>(move |_| refreshed());
        cx.bind::<SyncState>(move |_| refresh());
        let actions = cx.dispatch::<Board>();
        let syncing = cx.dispatch::<SyncState>();
        root.global::<TodoUi>()
            .on_command(move |command| match command {
                0 => actions.emit(Undo),
                1 => actions.emit(ClearCompleted),
                2 => actions.emit(Import(sample_titles())),
                3 => actions.emit(StopImport),
                4 => syncing.emit(SyncNow),
                _ => syncing.emit(CancelSync),
            });
    }
}

#[derive(Default)]
struct Tasks;

#[page]
impl Page for Tasks {
    fn install(cx: &Install) -> anyhow::Result<Self> {
        cx.install::<Search>(&())?;
        Ok(Self)
    }

    fn bind(&mut self, cx: &BindCx<Self>) {
        let root = cx.root::<AppWindow>();
        let board = cx.binding::<Board>();
        let search = cx.binding::<SearchResults>();
        let updating = root.clone_strong();
        let update = Rc::new(move || {
            let todos = visible(&board.get().0, &search.get(), Filter::All)
                .into_iter()
                .map(|todo| TodoRow {
                    id: todo.id.to_string().into(),
                    title: todo.title.into(),
                    completed: todo.done,
                })
                .collect::<Vec<_>>();
            updating
                .global::<TodoUi>()
                .set_todos(ModelRc::new(VecModel::from(todos)));
        });
        let updated = update.clone();
        cx.bind::<Board>(move |_| updated());
        cx.bind::<SearchResults>(move |_| update());
        let actions = cx.dispatch::<Board>();
        let adding = root.clone_strong();
        root.global::<TodoUi>().on_add(move |title| {
            actions.emit(Add(title.to_string()));
            adding.global::<TodoUi>().set_title(SharedString::default());
        });
        let searching = cx.dispatch::<SearchResults>();
        root.global::<TodoUi>()
            .on_search(move |query| searching.emit(Query(query.to_string())));
        root.global::<TodoUi>().on_toggle(move |id| {
            if let Ok(id) = id.parse() {
                actions.emit(Toggle(id));
            }
        });
        root.global::<TodoUi>().on_remove(move |id| {
            if let Ok(id) = id.parse() {
                actions.emit(Remove(id));
            }
        });
        let nav = cx.navigate::<Screen>();
        root.global::<TodoUi>().on_edit(move |id| {
            if let Ok(id) = id.parse() {
                nav.to(Screen::Detail { id });
            }
        });
    }
}

struct Details {
    id: u64,
}

#[page]
impl Page for Details {
    type Params = u64;

    fn install(cx: &Install<u64>) -> anyhow::Result<Self> {
        Ok(Self { id: *cx.params() })
    }

    fn bind(&mut self, cx: &BindCx<Self>) {
        let root = cx.root::<AppWindow>();
        let (board, actions) = cx.read::<Board>();
        let title = board
            .0
            .todos
            .iter()
            .find(|todo| todo.id == self.id)
            .map(|todo| todo.title.clone())
            .unwrap_or_default();
        root.global::<TodoUi>().set_detail_title(title.into());
        let id = self.id;
        let nav = cx.navigate::<Screen>();
        root.global::<TodoUi>().on_rename(move |title| {
            if !title.trim().is_empty() {
                actions.emit(Rename(id, title.to_string()));
                nav.back();
            }
        });
    }
}

#[derive(Default)]
struct Journal;

#[page]
impl Page for Journal {
    fn bind(&mut self, cx: &BindCx<Self>) {
        let root = cx.root::<AppWindow>();
        cx.bind_to::<Activity, _>(&root, |root, activity| {
            root.global::<TodoUi>()
                .set_entries(ModelRc::new(VecModel::from(
                    activity
                        .0
                        .iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )));
        });
    }
}
