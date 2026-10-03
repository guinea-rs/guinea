//! A page with nothing to say.
//!
//! No state, no messages, and nothing written down about either - `#[page]`
//! fills in `Message` and the empty `update`, and the struct being the state
//! means a page that keeps nothing is a unit struct. What is left is the whole
//! of what this page is: where it sits, what it installs, what it draws.

use guinea::feature::FeatureInitContext;
use guinea::iced::{Element, Page, PageCx, page};
use iced::Length::Fill;
use iced::widget::{column, scrollable, text};

use processes_core::services::ServicesFeature;
use processes_core::services::contracts::Services as Running;

#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    type Params = crate::routes::ServicesParams;
    type Installs = ServicesFeature;

    fn install(
        ctx: &FeatureInitContext,
        _params: &Self::Params,
    ) -> anyhow::Result<ServicesFeature> {
        ctx.install::<ServicesFeature>(&())
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Self::Message> {
        let (services, _) = cx.read::<Running, _>();

        let rows = services
            .items
            .iter()
            .map(|item| text(item.clone()).into())
            .collect::<Vec<Element<Self::Message>>>();

        scrollable(column(rows).spacing(4).padding(8))
            .height(Fill)
            .into()
    }
}
