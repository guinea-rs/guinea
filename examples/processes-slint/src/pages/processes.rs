use guinea::feature::FeatureInitContext;
use guinea::slint::{Page, PageCx};
use slint::ComponentHandle;

use processes_core::processes::contracts::{Kill, Processes as Running};
use processes_core::processes::pid_at;

use crate::ui::{AppWindow, ProcessesModel};

pub struct Processes;

impl Page for Processes {
    type Params = crate::routes::ProcessesParams;

    type Installs = processes_core::processes::ProcessesFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn bind(cx: PageCx<Self>) {
        let root = cx.root::<AppWindow>();
        let model = root.global::<ProcessesModel>();

        // Set once: the model reads the reducer's state and converts the rows
        // Slint actually asks for, so a refresh costs nothing here.
        model.set_items(cx.rows::<Running, _>(|state| &state.items));

        // Read at click time rather than captured: the actor refreshes the
        // list, and the row under the button may not be the row that was there
        // when the page was installed.
        let binding = cx.binding::<Running>();
        model.on_kill(move |index| {
            let pid = pid_at(&binding.peek().items, index as usize);
            if let Some(pid) = pid {
                binding.dispatch().emit(Kill(pid));
            }
        });
    }
}
