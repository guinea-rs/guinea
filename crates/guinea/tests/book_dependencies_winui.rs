#![cfg(all(windows, feature = "winui"))]

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::Mounted;
use guinea::winui::reactor::{StackPanel, TextBlock, View};
use guinea::winui::{Layout, LayoutCx, Page, PageCx, layout, page};

#[path = "support/dependency_book.rs"]
mod model;

use model::*;

//@show dependencies: layout installation
struct WorkspaceLayout {
    session: Session,
    document: Document,
}

#[layout]
impl Layout for WorkspaceLayout {
    fn install(cx: &Install, session: Session) -> anyhow::Result<Self> {
        let Editor(document) = cx.install::<Editor>(&())?;
        Ok(Self { session, document })
    }
    //@hide
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((
                TextBlock::new().text(format!(
                    "Workspace: {}: {}",
                    self.session.user,
                    self.document.title()
                )),
                cx.mount::<EditorPage>(&()),
                cx.mount::<PreviewPage>(&()),
            ))
            .into()
    }
    //@unhide
}
//@show-end

//@show dependencies: page installation
struct EditorPage {
    session: Session,
    document: Document,
}

#[page]
impl Page for EditorPage {
    fn install(_cx: &Install, session: Session, document: Document) -> anyhow::Result<Self> {
        Ok(Self { session, document })
    }
    //@hide
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new()
            .text(format!(
                "Editor: {}: {}",
                self.session.user,
                self.document.title()
            ))
            .into()
    }
    //@unhide
}
//@show-end

struct PreviewPage {
    session: Session,
    document: Document,
}

#[page]
impl Page for PreviewPage {
    fn install(_cx: &Install, session: Session, document: Document) -> anyhow::Result<Self> {
        Ok(Self { session, document })
    }

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new()
            .text(format!(
                "Preview: {}: {}",
                self.session.user,
                self.document.title()
            ))
            .into()
    }
}

#[guinea::test]
fn layout_page_and_child_receive_their_declared_inputs(h: &mut Harness) {
    guinea::app::validate_declarations();
    let store = h.app(app).unwrap();
    store.rename("Draft");
    let layout = Mounted::<WorkspaceLayout>::mount(h.segment(), ()).unwrap();
    assert!(layout.find_text("Workspace: Alice: Draft").is_some());
    assert!(layout.find_text("Editor: Alice: Draft").is_some());
    assert!(layout.find_text("Preview: Alice: Draft").is_some());
}
