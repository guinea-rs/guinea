//! A form, which is the shape Elm is best at and the one this framework had
//! nothing to show yet.
//!
//! Everything on it is the page's own: two fields being typed into are no
//! business of the domain, and no actor is involved until there is something
//! to submit. So there is no reducer here at all - the page is its state, and
//! `update` is the only place it changes.
//!
//! It accepts anything: this demonstrates the form, not authentication.

use guinea::iced::{Element, Page, PageCx, UpdateCx, page};
use iced::Length::Fill;
use iced::widget::{button, center, column, container, row, text, text_input};

#[derive(Default)]
pub struct Login {
    user: String,
    password: String,
    /// Who is signed in, once someone is. Local because nothing else in the
    /// application asks - the moment something does, this becomes a reducer an
    /// actor drives, and the page stops holding it.
    signed_in: Option<String>,
}

#[derive(Clone)]
pub enum Msg {
    User(String),
    Password(String),
    Submit,
    SignOut,
}

#[page]
impl Page for Login {
    type Params = crate::routes::LoginParams;
    type Message = Msg;

    fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Msg::User(user) => self.user = user,
            Msg::Password(password) => self.password = password,
            Msg::Submit => {
                self.signed_in = Some(std::mem::take(&mut self.user));
                self.password.clear();
            }
            Msg::SignOut => self.signed_in = None,
        }
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, Msg> {
        let form = match &self.signed_in {
            Some(user) => column![
                text(format!("Signed in as {user}")).size(18),
                button(text("Sign out")).on_press(Msg::SignOut),
            ],

            None => {
                let ready = !self.user.is_empty() && !self.password.is_empty();

                column![
                    text("Sign in").size(18),
                    text_input("User", &self.user)
                        .on_input(Msg::User)
                        // Enter submits, and only while the form is complete -
                        // the same condition the button is enabled by, said
                        // once.
                        .on_submit_maybe(ready.then_some(Msg::Submit)),
                    text_input("Password", &self.password)
                        .secure(true)
                        .on_input(Msg::Password)
                        .on_submit_maybe(ready.then_some(Msg::Submit)),
                    row![button(text("Sign in")).on_press_maybe(ready.then_some(Msg::Submit))],
                ]
            }
        };

        center(
            container(form.spacing(12).width(280))
                .padding(20)
                .style(container::bordered_box),
        )
        .height(Fill)
        .into()
    }
}
