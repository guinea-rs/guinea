//! An address and the route it came from: what `link` writes, `parse` reads
//! back as the same route.

use guinea_router::navigator::Routable as _;

#[derive(Clone, Debug, PartialEq, guinea_macros::Routable)]
enum Place {
    #[at("/files")]
    Files,
    #[at("/files/:path")]
    File { path: String },
    #[at("/folder/:name/open")]
    Folder { name: String },
    #[at("/kind/:type")]
    Kind { r#type: String },
}

#[test]
fn a_capture_named_like_a_keyword_lands_in_its_raw_field() {
    let route = Place::Kind {
        r#type: "disk".to_string(),
    };

    assert_eq!(route.link().as_deref(), Some("/kind/disk"));
    assert_eq!(Place::parse("/kind/disk"), Some(route));
}

#[test]
fn the_manifest_names_a_keyword_capture_as_the_pattern_wrote_it() {
    let links = Place::deep_links();
    let kind = links.iter().find(|link| link.route == "Kind");

    assert_eq!(kind.map(|kind| kind.captures[0].name), Some("type"));
}

#[test]
fn an_empty_capture_at_the_end_parses_back_to_its_own_route() {
    let route = Place::File { path: String::new() };
    let link = route.link().unwrap_or_default();

    assert_eq!(Place::parse(&link), Some(route), "the link was {link:?}");
}

#[test]
fn an_empty_capture_in_the_middle_parses_back_to_its_own_route() {
    let route = Place::Folder { name: String::new() };
    let link = route.link().unwrap_or_default();

    assert_eq!(Place::parse(&link), Some(route), "the link was {link:?}");
}

#[test]
fn an_address_without_its_trailing_slash_is_the_page_it_names() {
    assert_eq!(Place::parse("/files"), Some(Place::Files));
    assert_eq!(Place::parse("files"), Some(Place::Files), "the leading slash is optional");
}

#[test]
fn a_trailing_slash_nothing_captures_is_still_the_same_page() {
    assert_eq!(
        Place::parse("/folder/disk/open/"),
        Some(Place::Folder { name: "disk".to_string() })
    );
}

#[test]
fn an_empty_segment_in_the_middle_of_an_address_is_not_skipped() {
    assert_eq!(Place::parse("/folder//disk/open"), None);
}
