#![cfg(all(feature = "eframe", feature = "harness"))]

use guinea::app::Harness;
use guinea::eframe::harness::Mounted;
use guinea::eframe::{Layout, LayoutCx, Page, PageCx, Window, layout, page};
use guinea::prelude::*;

#[path = "support/dependency_book.rs"]
mod model;

use model::*;

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

    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        cx.ui().label(format!(
            "Workspace: {}: {}",
            self.session.user,
            self.document.title()
        ));
        let page = cx.mount::<EditorPage>(&());
        page.draw(cx.ui());
        let preview = cx.mount::<PreviewPage>(&());
        preview.draw(cx.ui());
    }
}

struct EditorPage {
    session: Session,
    document: Document,
}

#[page]
impl Page for EditorPage {
    fn install(_cx: &Install, session: Session, document: Document) -> anyhow::Result<Self> {
        Ok(Self { session, document })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        cx.ui().label(format!(
            "Editor: {}: {}",
            self.session.user,
            self.document.title()
        ));
    }
}

struct PreviewPage {
    session: Session,
    document: Document,
}

#[page]
impl Page for PreviewPage {
    fn install(_cx: &Install, session: Session, document: Document) -> anyhow::Result<Self> {
        Ok(Self { session, document })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        cx.ui().label(format!(
            "Preview: {}: {}",
            self.session.user,
            self.document.title()
        ));
    }
}

#[guinea::test]
fn layout_page_and_child_receive_their_declared_inputs(h: &mut Harness) {
    guinea::app::validate_declarations();
    let store = h.app(app).unwrap();
    store.rename("Draft");
    let layout = Mounted::window(h, Window::of::<WorkspaceLayout>());
    assert!(layout.has_text("Workspace: Alice: Draft"));
    assert!(layout.has_text("Editor: Alice: Draft"));
    assert!(layout.has_text("Preview: Alice: Draft"));
}
