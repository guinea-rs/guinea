use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::Harness;
use guinea::prelude::*;

#[path = "support/dependency_book.rs"]
mod model;

use model::*;

#[derive(Clone, Debug, Default, PartialEq)]
struct PreviewText(String);

#[reducer]
fn preview_text(this: &mut PreviewText, value: String) {
    this.0 = value;
}

feature! { Preview { exports { PreviewText } } }

#[installs]
fn preview(cx: &Install, session: Session, document: Document) -> anyhow::Result<Preview> {
    let text = cx
        .state::<PreviewText>()
        .seed(PreviewText(format!(
            "{}: {}",
            session.user,
            document.title()
        )))
        .plain();
    Ok(Preview(text))
}

#[test]
fn declarations_form_a_valid_graph() {
    guinea::app::validate_declarations();
}

#[guinea::test]
fn outputs_reach_descendants_and_share_the_same_store(h: &mut Harness) {
    let store = h.app(app).unwrap();
    let editor = h.child();
    editor.install::<Editor>(&()).unwrap();
    store.rename("Edited notes");
    let child = editor.child();
    child.install::<Preview>(&()).unwrap();
    assert_eq!(child.state::<PreviewText>().0, "Alice: Edited notes");
}

#[guinea::test]
fn a_sibling_cannot_consume_the_editors_document(h: &mut Harness) {
    h.app(app).unwrap();
    let editor = h.child();
    editor.install::<Editor>(&()).unwrap();
    let sibling = h.child();
    let panic = catch_unwind(AssertUnwindSafe(|| sibling.install::<Preview>(&())));
    assert!(panic.is_err());
}

#[guinea::test]
fn removing_the_provider_scope_makes_its_output_unavailable(h: &mut Harness) {
    h.app(app).unwrap();
    let editor = h.child();
    editor.install::<Editor>(&()).unwrap();
    editor.leave();
    let replacement = h.child();
    let panic = catch_unwind(AssertUnwindSafe(|| replacement.install::<Preview>(&())));
    assert!(panic.is_err());
}

#[guinea::test]
fn failed_resource_creation_returns_an_error(h: &mut Harness) {
    assert!(h.plugin::<Storage>(StorageConfig("")).is_err());
}
