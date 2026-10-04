use guinea::feature::FeatureInitContext;
use guinea::slint::{Page, PageCx};
use slint::ComponentHandle;

use processes_core::services::contracts::Services as Running;

use crate::ui::{AppWindow, ServicesModel};

pub struct Services;

impl Page for Services {
    type Params = crate::routes::ServicesParams;

    type Installs = processes_core::services::ServicesFeature;

    fn install(
        ctx: &FeatureInitContext,
        _params: &Self::Params,
    ) -> anyhow::Result<Self::Installs> {
        ctx.install(&())
    }

    fn bind(cx: PageCx<Self>) {
        cx.root::<AppWindow>()
            .global::<ServicesModel>()
            .set_items(cx.rows::<Running, _>(|state| &state.items));
    }
}
