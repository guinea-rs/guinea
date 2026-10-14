//! An address and the route it came from: what `link` writes, `parse` reads
//! back as the same route.

use guinea_app::feature::FeatureInitContext;
use guinea_macros::routes;
use guinea_router::headless::{HeadlessCx, Page};

macro_rules! page {
    ($name:ident, $params:ty) => {
        pub struct $name;

        impl Page for $name {
            type Params = $params;
            type Installs = ();

            fn install(_ctx: &FeatureInitContext, _params: &$params) -> anyhow::Result<()> {
                Ok(())
            }

            fn view(_cx: &mut HeadlessCx<Self>) {}
        }
    };
}

page!(Files, ());
page!(File, FileParams);
page!(Folder, FolderParams);
page!(Kind, KindParams);

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        page(Files) link("/files")
        page(File) link("/files/:path") { path: String }
        page(Folder) link("/folder/:name/open") { name: String }
        page(Kind) link("/kind/:type") { r#type: String }
    }
}

#[test]
fn a_capture_named_like_a_keyword_lands_in_its_raw_field() {
    let route = Route::Kind {
        r#type: "disk".to_string(),
    };

    assert_eq!(route.link().as_deref(), Some("/kind/disk"));
    assert_eq!(Route::parse("/kind/disk"), Some(route));
}

#[test]
fn the_manifest_names_a_keyword_capture_as_the_pattern_wrote_it() {
    let surface = Route::deep_links();
    let kind = surface
        .iter()
        .find(|link| link.route == "Kind")
        .expect("Kind is addressable");

    assert_eq!(kind.captures[0].name, "type");
}

#[test]
fn an_empty_capture_at_the_end_parses_back_to_its_own_route() {
    let route = Route::File { path: String::new() };
    let link = route.link().expect("an addressable route");

    assert_eq!(Route::parse(&link), Some(route), "the link was {link:?}");
}

#[test]
fn an_empty_capture_in_the_middle_parses_back_to_its_own_route() {
    let route = Route::Folder { name: String::new() };
    let link = route.link().expect("an addressable route");

    assert_eq!(Route::parse(&link), Some(route), "the link was {link:?}");
}

#[test]
fn an_address_without_its_trailing_slash_is_the_page_it_names() {
    assert_eq!(Route::parse("/files"), Some(Route::Files {}));
    assert_eq!(Route::parse("files"), Some(Route::Files {}), "the leading slash is optional");
}

#[test]
fn a_trailing_slash_nothing_captures_is_still_the_same_page() {
    assert_eq!(
        Route::parse("/folder/disk/open/"),
        Some(Route::Folder { name: "disk".to_string() })
    );
}

#[test]
fn an_empty_segment_in_the_middle_of_an_address_is_not_skipped() {
    assert_eq!(Route::parse("/folder//disk/open"), None);
}
