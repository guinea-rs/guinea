//! The page that minds being left - and the one that proves a view may borrow.
//!
//! `text_editor` keeps a `&Content` for the life of the element, so this page
//! is only expressible because a view is `Element<'_, Msg>` borrowed from the
//! node rather than something the node had to hand over by value. The node
//! lives in the shell for exactly that reason; see [`guinea::iced::Nodes`].
//!
//! `leaving` is the guard: a method on the node, answered from the node's own
//! state, because on the way out the state exists and is the only thing that
//! knows whether there is anything to lose. Entering is the asymmetric case -
//! there is nothing to read yet - which is why an enter guard belongs in the
//! route declaration instead.
//!
//! Switching tabs with something typed here puts the question up. The
//! navigation is parked, not cancelled: nothing has been torn down, so
//! answering "Discard" finishes the move that was already asked for, and
//! switching to a third tab while the question is open replaces it.

use guinea::iced::{Ask, Element, Page, PageCx, UpdateCx, Verdict, page};
use iced::Length::Fill;
use iced::widget::{button, column, row, text, text_editor};

#[derive(Default)]
pub struct Draft {
    note: text_editor::Content,
}

#[derive(Clone)]
pub enum Msg {
    Edit(text_editor::Action),
    Clear,
}

impl Draft {
    fn written(&self) -> String {
        self.note.text().trim().to_string()
    }
}

#[page]
impl Page for Draft {
    type Params = crate::routes::DraftParams;
    type Message = Msg;

    fn leaving(&self) -> Verdict {
        let written = self.written();
        if written.is_empty() {
            return Verdict::Allow;
        }

        // Built here rather than at install: by the time it is asked, the
        // language may have changed and so may what the text names.
        Verdict::ask(Ask::new(
            format!(
                "{} unsaved characters over {} line(s). Leave anyway?",
                written.len(),
                self.note.line_count()
            ),
            "Discard",
            "Keep editing",
        ))
    }

    fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Msg::Edit(action) => self.note.perform(action),
            Msg::Clear => self.note = text_editor::Content::new(),
        }
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, Msg> {
        let dirty = !self.written().is_empty();

        column![
            row![
                text("Type something, then switch tabs.").width(Fill),
                text(if dirty {
                    "leaving will ask"
                } else {
                    "leaving is free"
                }),
                button(text("Clear")).on_press_maybe(dirty.then_some(Msg::Clear)),
            ]
            .spacing(8),
            // The widget that made all of this necessary: it holds the
            // `Content` rather than a copy of its text.
            text_editor(&self.note)
                .on_action(Msg::Edit)
                .height(Fill)
                .placeholder("nobody will save this"),
        ]
        .spacing(12)
        .padding(12)
        .into()
    }
}
