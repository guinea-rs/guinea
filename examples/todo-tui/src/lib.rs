use guinea::prelude::*;
use guinea::ratatui::{Handled, InputCx, Layout, LayoutCx, Page, PageCx, Window, layout, page};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout as Areas};
use ratatui::widgets::{Block, Borders, Paragraph};
use todo_core::*;

pub fn window() -> Window {
    Window::of::<Shell>().at(Screen::Tasks {
        filter: Filter::All,
    })
}

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn render(&mut self, cx: &mut LayoutCx<'_, '_, Self>) {
        let nav = cx.navigate::<Screen>();
        let summary = cx.mount::<Summary>(&());
        let content = cx.outlet(&nav, |route, cx| match route {
            Screen::Tasks { filter } => cx.mount::<Tasks>(filter).keep(),
            Screen::Detail { id } => cx.mount::<Details>(id),
            Screen::Activity => cx.mount::<Journal>(&()),
        });
        let areas = Areas::vertical([Constraint::Length(7), Constraint::Min(1)]).split(cx.area());
        summary.draw(cx.frame(), areas[0]);
        content.draw(cx.frame(), areas[1]);
    }

    fn on_key(&mut self, cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        let nav = cx.navigate::<Screen>();
        let actions = cx.dispatch::<Board>();
        match key.code {
            KeyCode::Char('1') => {
                nav.to(Screen::Tasks {
                    filter: Filter::All,
                });
            }
            KeyCode::Char('2') => {
                nav.to(Screen::Tasks {
                    filter: Filter::Active,
                });
            }
            KeyCode::Char('3') => {
                nav.to(Screen::Tasks {
                    filter: Filter::Completed,
                });
            }
            KeyCode::Char('l') => {
                nav.to(Screen::Activity);
            }
            KeyCode::Char('i') => actions.emit(Import(sample_titles())),
            KeyCode::Char('x') => actions.emit(StopImport),
            KeyCode::Char('u') => actions.emit(Undo),
            KeyCode::Char('c') => actions.emit(ClearCompleted),
            KeyCode::Char('s') => cx.dispatch::<SyncState>().emit(SyncNow),
            KeyCode::Char('z') => cx.dispatch::<SyncState>().emit(CancelSync),
            KeyCode::Esc => {
                if !nav.back() {
                    cx.close();
                }
            }
            KeyCode::Char('q') => cx.close(),
            _ => return Handled::No,
        }
        Handled::Yes
    }
}

#[derive(Default)]
struct Summary;

#[page]
impl Page for Summary {
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let (board, _) = cx.read::<Board>();
        let (save, _) = cx.read::<SaveState>();
        let (sync, _) = cx.read::<SyncState>();
        let (clock, _) = cx.read::<Clock>();
        let text = format!(
            "{}\n{}\n1 all · 2 active · 3 completed · l activity · n new · / search\nu undo · c clear · i import · x stop import · s sync · z cancel sync · q quit\nSession: {} seconds",
            board.0.summary(),
            status(&save, &sync),
            clock.0
        );
        let area = cx.area();
        cx.frame().render_widget(
            Paragraph::new(text).block(
                Block::default()
                    .title("guinea · todo")
                    .borders(Borders::ALL),
            ),
            area,
        );
    }
}

enum Editing {
    None,
    Title,
    Query,
}

struct Tasks {
    filter: Filter,
    selected: usize,
    title: String,
    query: String,
    editing: Editing,
}

#[page]
impl Page for Tasks {
    type Params = Filter;

    fn install(cx: &Install<Filter>) -> anyhow::Result<Self> {
        cx.install::<Search>(&())?;
        Ok(Self {
            filter: *cx.params(),
            selected: 0,
            title: String::new(),
            query: String::new(),
            editing: Editing::None,
        })
    }

    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let (board, _) = cx.read::<Board>();
        let (search, _) = cx.read::<SearchResults>();
        let todos = visible(&board.0, &search, self.filter);
        self.selected = self.selected.min(todos.len().saturating_sub(1));
        let mut text = format!(
            "New: {}\nSearch: {}{}\n↑↓ select · Space toggle · Enter edit · Delete remove\n\n",
            self.title,
            self.query,
            if search.pending {
                " (searching…)"
            } else {
                ""
            }
        );
        text.push_str(&task_lines(&todos, self.selected).join("\n"));
        let area = cx.area();
        cx.frame().render_widget(
            Paragraph::new(text).block(
                Block::default()
                    .title(format!("{:?}", self.filter))
                    .borders(Borders::ALL),
            ),
            area,
        );
    }

    fn on_key(&mut self, cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        if !matches!(self.editing, Editing::None) {
            let text = match self.editing {
                Editing::Title => &mut self.title,
                _ => &mut self.query,
            };
            match key.code {
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    text.push(character)
                }
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Esc => {
                    self.editing = Editing::None;
                    return Handled::Yes;
                }
                KeyCode::Enter => {
                    if matches!(self.editing, Editing::Title) {
                        cx.dispatch::<Board>()
                            .emit(Add(std::mem::take(&mut self.title)));
                    }
                    self.editing = Editing::None;
                    return Handled::Yes;
                }
                _ => return Handled::Yes,
            }
            if matches!(self.editing, Editing::Query) {
                cx.dispatch::<SearchResults>()
                    .emit(Query(self.query.clone()));
            }
            return Handled::Yes;
        }
        let (board, actions) = cx.read::<Board>();
        let (search, _) = cx.read::<SearchResults>();
        let todos = visible(&board.0, &search, self.filter);
        let selected = todos.get(self.selected);
        match key.code {
            KeyCode::Char('n') => self.editing = Editing::Title,
            KeyCode::Char('/') => self.editing = Editing::Query,
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(todos.len().saturating_sub(1)),
            KeyCode::Char(' ') => {
                if let Some(todo) = selected {
                    actions.emit(Toggle(todo.id));
                }
            }
            KeyCode::Delete => {
                if let Some(todo) = selected {
                    actions.emit(Remove(todo.id));
                }
            }
            KeyCode::Enter => {
                if let Some(todo) = selected {
                    cx.navigate::<Screen>().to(Screen::Detail { id: todo.id });
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }
}

pub fn task_lines(todos: &[Todo], selected: usize) -> Vec<String> {
    todos
        .iter()
        .enumerate()
        .map(|(index, todo)| {
            format!(
                "{} [{}] {}",
                if index == selected { '>' } else { ' ' },
                if todo.done { 'x' } else { ' ' },
                todo.title
            )
        })
        .collect()
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

    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let area = cx.area();
        cx.frame().render_widget(
            Paragraph::new(format!("{}\n\nEnter saves · Esc discards", self.title)).block(
                Block::default()
                    .title(format!("Task {}", self.id))
                    .borders(Borders::ALL),
            ),
            area,
        );
    }

    fn on_key(&mut self, cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        match key.code {
            KeyCode::Char(character) => self.title.push(character),
            KeyCode::Backspace => {
                self.title.pop();
            }
            KeyCode::Enter if !self.title.trim().is_empty() => {
                cx.dispatch::<Board>()
                    .emit(Rename(self.id, self.title.clone()));
                cx.navigate::<Screen>().back();
            }
            KeyCode::Esc => {
                cx.navigate::<Screen>().back();
            }
            _ => {}
        }
        Handled::Yes
    }
}

#[derive(Default)]
struct Journal;

#[page]
impl Page for Journal {
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let (activity, _) = cx.read::<Activity>();
        let area = cx.area();
        cx.frame().render_widget(
            Paragraph::new(activity.0.join("\n"))
                .block(Block::default().title("Activity log").borders(Borders::ALL)),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_rows_preserve_completion_and_selection() {
        let todos = vec![
            Todo {
                id: 1,
                title: "first".into(),
                done: false,
            },
            Todo {
                id: 2,
                title: "second".into(),
                done: true,
            },
        ];
        assert_eq!(task_lines(&todos, 1), ["  [ ] first", "> [x] second"]);
        assert!(task_lines(&[], 0).is_empty());
    }
}
