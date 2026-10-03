//! The page the whole adapter exists to make possible.
//!
//! The struct is the page and the page is its state - one declaration for
//! where it sits in the route tree, what it captured, what it keeps, what can
//! happen to it and how it draws.
//!
//! `list_replaced` is the part Elm has no answer for. The selected row is an
//! index into a list an actor owns, and killing the last process leaves the
//! index past the end unless something says so.

use guinea::feature::FeatureInitContext;
use guinea::iced::{Element, Observing, Page, PageCx, UpdateCx, page};
use iced::Length::Fill;
use iced::widget::{button, column, row, scrollable, text};

use processes_core::processes::ProcessesFeature;
use processes_core::processes::contracts::{Kill, Listed, Processes as Running};
use processes_core::processes::pid_at;

/// Everything about this page the domain has no opinion about.
#[derive(Default)]
pub struct Processes {
    row: usize,
}

// `Clone` because iced's own widgets ask for it - `button::on_press` holds the
// message and hands out copies. The framework does not require it: a node
// whose widgets never do is free to leave it off.
#[derive(Clone)]
pub enum Msg {
    Select(usize),
    KillSelected,
    /// The list was replaced under the selection. Not "the list changed" - the
    /// new length, which is the only part the selection cares about.
    ListLength(usize),
}

/// A translation, not a mutation: it produces a message, and `update` stays
/// the one place this page changes.
fn list_replaced(update: &Listed) -> Option<Msg> {
    let Listed::Items(items) = update;
    Some(Msg::ListLength(items.len()))
}

#[page]
impl Page for Processes {
    type Params = crate::routes::ProcessesParams;
    type Message = Msg;
    type Installs = ProcessesFeature;

    fn install(
        ctx: &FeatureInitContext,
        params: &Self::Params,
    ) -> anyhow::Result<ProcessesFeature> {
        ctx.install::<ProcessesFeature>(&params.context)
    }

    fn observes(cx: &Observing<'_, Msg>) {
        cx.on::<Running>(list_replaced);
    }

    fn update(&mut self, message: Msg, cx: &mut UpdateCx<'_, Self>) {
        match message {
            Msg::Select(row) => self.row = row,

            // No `Task`: killing a process is not this node's work, it is the
            // actor's. The page says what it wants and goes back to drawing;
            // what came of it arrives as an update to the reducer.
            Msg::KillSelected => {
                let (processes, dispatch) = cx.read::<Running, _>();
                if let Some(pid) = pid_at(&processes.items, self.row) {
                    dispatch.emit(Kill(pid));
                }
            }

            Msg::ListLength(len) => self.row = self.row.min(len.saturating_sub(1)),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Msg> {
        let (processes, _) = cx.read::<Running, _>();

        let rows = processes
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let selected = index == self.row;
                let label = button(text(format!("{} {item}", index + 1)))
                    .width(Fill)
                    .style(if selected {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(Msg::Select(index));

                let kill = button(text("Kill"))
                    .style(button::danger)
                    // Only the selected row can act, so the pid is read at
                    // update time from the selection - never captured here,
                    // where it would go stale the moment the actor refreshes.
                    .on_press_maybe(selected.then_some(Msg::KillSelected));

                row![label, kill].spacing(8).into()
            })
            .collect::<Vec<Element<Msg>>>();

        scrollable(column(rows).spacing(4).padding(8))
            .height(Fill)
            .into()
    }
}
