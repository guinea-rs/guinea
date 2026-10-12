//! Path matching as a prefix tree.
//!
//! A route declaration is a list of patterns, and the obvious way to match a
//! path against them is to try each in turn. That makes declaration order load
//! bearing: a pattern that begins with a capture accepts any first segment, so
//! it swallows every route of the same length declared below it.
//!
//! Here the patterns are folded into a tree instead - one node per shared
//! prefix - and a path walks it. Order survives only where it has to: at any
//! one node the literal branches are tried before the capture branches, and
//! literals among themselves in the order they were declared. Between levels
//! it means nothing at all.
//!
//! Two patterns of the same shape - same length, same literals in the same
//! places, captures in the same places - would sit on the same node, and no
//! ordering rule can decide between them. That is a [`Conflict`], reported
//! rather than silently resolved.

use std::fmt;

use proc_macro2::{Ident, Literal, TokenStream};
use quote::{format_ident, quote};
use syn::ext::IdentExt;

use crate::{Leaf, Segment, parse_pattern, type_ident};

/// One step down the tree: what a path segment has to be to take it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Branch {
    Literal(String),
    Capture { name: String },
}

/// The page a path ending at a node builds.
#[derive(Clone, Debug)]
pub struct MatchLeaf {
    /// The page type, as the declaration wrote it.
    pub ty: syn::Type,
    /// The route enum's variant for that page.
    pub variant: Ident,
    /// The variant's fields, in declaration order.
    pub fields: Vec<Ident>,
}

/// A node of the match tree.
#[derive(Clone, Debug, Default)]
pub struct MatchNode {
    literals: Vec<(String, MatchNode)>,
    captures: Vec<(String, MatchNode)>,
    leaf: Option<MatchLeaf>,
}

impl MatchNode {
    /// The branches out of this node, in the order they are to be tried:
    /// literals first, in declaration order, then captures.
    pub fn children(&self) -> Vec<(Branch, &MatchNode)> {
        self.literals
            .iter()
            .map(|(lit, node)| (Branch::Literal(lit.clone()), node))
            .chain(
                self.captures
                    .iter()
                    .map(|(name, node)| (Branch::Capture { name: name.clone() }, node)),
            )
            .collect()
    }

    /// The page a path ending here builds, if any.
    pub fn leaf(&self) -> Option<&MatchLeaf> {
        self.leaf.as_ref()
    }

    fn is_empty(&self) -> bool {
        self.literals.is_empty() && self.captures.is_empty() && self.leaf.is_none()
    }

    fn literal_mut(&mut self, lit: &str) -> &mut MatchNode {
        let index = match self.literals.iter().position(|(name, _)| name == lit) {
            Some(index) => index,
            None => {
                self.literals.push((lit.to_string(), MatchNode::default()));
                self.literals.len() - 1
            }
        };
        &mut self.literals[index].1
    }

    fn capture_mut(&mut self, capture: &str) -> &mut MatchNode {
        let index = match self.captures.iter().position(|(name, _)| name == capture) {
            Some(index) => index,
            None => {
                self.captures
                    .push((capture.to_string(), MatchNode::default()));
                self.captures.len() - 1
            }
        };
        &mut self.captures[index].1
    }
}

/// Every declared pattern, folded into one tree.
#[derive(Clone, Debug)]
pub struct MatchTree {
    root: MatchNode,
}

impl MatchTree {
    /// The node an empty path ends at.
    pub fn root(&self) -> &MatchNode {
        &self.root
    }
}

/// Two pages no ordering rule can tell apart.
#[derive(Clone, Debug)]
pub struct Conflict {
    /// The page that claimed the shape first.
    pub first: syn::Type,
    /// The page that claimed it again.
    pub second: syn::Type,
    /// The shape both have, with captures written as `*`.
    pub shape: String,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "routes!: `{}` and `{}` both match `{}` - no path can tell them apart",
            type_ident(&self.first),
            type_ident(&self.second),
            self.shape
        )
    }
}

/// A page whose captures and fields disagree.
#[derive(Clone, Debug)]
pub enum FieldError {
    /// The pattern captures a segment the page has no field for.
    CaptureWithoutField { page: syn::Type, capture: String },
    /// The page declares a field its pattern never captures.
    FieldWithoutCapture { page: syn::Type, field: String },
    /// The pattern captures two segments under one name.
    RepeatedCapture { page: syn::Type, capture: String },
    /// A `~` field on a route that has to be reconstructed whole.
    LooseFieldOnOutwardRoute {
        page: syn::Type,
        field: String,
        tier: &'static str,
    },
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldError::CaptureWithoutField { page, capture } => write!(
                f,
                "routes!: `{}` captures `:{}` but declares no field `{}` to put it in",
                type_ident(page),
                capture,
                capture
            ),
            FieldError::FieldWithoutCapture { page, field } => write!(
                f,
                "routes!: `{}` declares the field `{}`, but its pattern captures no `:{}`",
                type_ident(page),
                field,
                field
            ),
            FieldError::RepeatedCapture { page, capture } => write!(
                f,
                "routes!: `{}` captures `:{}` twice - one field holds one segment, so each \
                 capture needs a name of its own",
                type_ident(page),
                capture
            ),
            FieldError::LooseFieldOnOutwardRoute { page, field, tier } => write!(
                f,
                "routes!: `{}` is `{}`, so it has to be reconstructible from the outside - \
                 and `~{}` is a field that was never kept. Drop the `~`, or drop the `{}`.",
                type_ident(page),
                tier,
                field,
                tier
            ),
        }
    }
}

/// Folds the declared pages into a match tree.
///
/// Fails with every ambiguity at once, rather than the first: a declaration
/// with two of them should not have to be fixed twice.
pub fn build(leaves: &[Leaf]) -> Result<MatchTree, Vec<Conflict>> {
    let mut root = MatchNode::default();
    let mut conflicts = Vec::new();
    let mut claimed: Vec<(String, syn::Type)> = Vec::new();

    for leaf in leaves {
        // Only what agreed to have an address is reachable from outside, so
        // only that goes in the tree.
        let Some(link) = &leaf.link else { continue };

        let segments = parse_pattern(link);
        let shape = shape_of(&segments);

        let first = claimed
            .iter()
            .find(|(seen, _)| *seen == shape)
            .map(|(_, ty)| ty.clone());
        if let Some(first) = first {
            conflicts.push(Conflict {
                first,
                second: leaf.ty.clone(),
                shape,
            });
            continue;
        }

        claimed.push((shape, leaf.ty.clone()));
        insert(&mut root, &segments, leaf);
    }

    if conflicts.is_empty() {
        Ok(MatchTree { root })
    } else {
        Err(conflicts)
    }
}

/// Checks that every capture has a field to land in, every field a capture
/// to fill it, and no two captures one name.
///
/// Left unchecked, either one reaches the compiler as an unresolved identifier
/// inside generated code, which says nothing about the declaration that caused
/// it.
pub fn check_fields(leaves: &[Leaf]) -> Vec<FieldError> {
    let mut errors = Vec::new();

    for leaf in leaves {
        // Both outward tiers have to rebuild a route whole - from a path, or
        // from text on disk - and a `~` field is one that was never kept.
        let tier = match (leaf.is_addressable(), leaf.restorable) {
            (true, _) => Some("link"),
            (_, true) => Some("restorable"),
            _ => None,
        };

        if let Some(tier) = tier {
            for loose in leaf.fields.iter().filter(|field| !field.identity) {
                errors.push(FieldError::LooseFieldOnOutwardRoute {
                    page: leaf.ty.clone(),
                    field: loose.name.to_string(),
                    tier,
                });
            }
        }

        // A page with no address captures nothing from one, so its fields are
        // its own business - they arrive by value, from whoever navigated.
        if !leaf.is_addressable() {
            continue;
        }

        let mut captures: Vec<String> = Vec::new();
        let mut repeated: Vec<String> = Vec::new();

        for capture in captures_of(leaf) {
            if !captures.contains(&capture) {
                captures.push(capture);
                continue;
            }

            if !repeated.contains(&capture) {
                errors.push(FieldError::RepeatedCapture {
                    page: leaf.ty.clone(),
                    capture: capture.clone(),
                });
                repeated.push(capture);
            }
        }

        for capture in &captures {
            if !leaf.fields.iter().any(|field| field.name.unraw() == capture) {
                errors.push(FieldError::CaptureWithoutField {
                    page: leaf.ty.clone(),
                    capture: capture.clone(),
                });
            }
        }

        for field in &leaf.fields {
            let name = field.name.unraw();
            if !captures.iter().any(|capture| name == capture) {
                errors.push(FieldError::FieldWithoutCapture {
                    page: leaf.ty.clone(),
                    field: name.to_string(),
                });
            }
        }
    }

    errors
}

/// Generates `pub fn parse(path: &str) -> Option<Self>` for the route enum.
///
/// The generated walk backtracks: a branch whose literal does not match, or
/// whose capture will not decode into the field's type, is simply the wrong
/// branch, and the next one is tried. Nothing inside it can end the search -
/// the function returns only on a whole path matched, or falls out of the tree
/// with `None`.
///
/// Every `/` after the leading one separates a segment, so an empty capture
/// keeps its place: `/files/` is `/files/:path` with an empty `path`. Only when
/// nothing matches that way is a trailing slash dropped and the walk tried
/// again, so `/folder/disk/open/` still reaches `/folder/:name/open`.
///
/// Expects [`check_fields`] to have passed; a field with no capture behind it
/// has nothing to bind, and panics here rather than reaching the compiler as a
/// stray identifier.
///
/// `link` is where the module holding `LinkValue` lives; the caller knows it
/// because it knows how the crate was named, and this crate does not.
pub fn emit_parse(tree: &MatchTree, enum_ident: &Ident, link: &TokenStream) -> TokenStream {
    if tree.root.is_empty() {
        return quote! {
            pub fn parse(path: &str) -> Option<Self> {
                let _ = path;
                None
            }
        };
    }

    let body = emit_node(&tree.root, 0, enum_ident, &mut Vec::new(), link);
    quote! {
        pub fn parse(path: &str) -> Option<Self> {
            let __whole: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
            let __walk = |__parts: &[&str]| -> Option<Self> {
                #body
                None
            };

            __walk(&__whole).or_else(|| match __whole.split_last() {
                Some((&"", __rest)) => __walk(__rest),
                _ => None,
            })
        }
    }
}

fn emit_node(
    node: &MatchNode,
    depth: usize,
    enum_ident: &Ident,
    bound: &mut Vec<(String, usize)>,
    link: &TokenStream,
) -> TokenStream {
    let mut out = TokenStream::new();
    let here = Literal::usize_unsuffixed(depth);
    let deeper = Literal::usize_unsuffixed(depth + 1);

    if let Some(leaf) = &node.leaf {
        let build = emit_leaf(leaf, enum_ident, bound, link);
        out.extend(quote! {
            if __parts.len() == #here {
                #build
            }
        });
    }

    for (lit, child) in &node.literals {
        let inner = emit_node(child, depth + 1, enum_ident, bound, link);
        out.extend(quote! {
            if __parts.len() >= #deeper && __parts[#here] == #lit {
                #inner
            }
        });
    }

    for (name, child) in &node.captures {
        let segment = format_ident!("__seg_{}", depth);
        bound.push((name.clone(), depth));
        let inner = emit_node(child, depth + 1, enum_ident, bound, link);
        bound.pop();
        out.extend(quote! {
            if __parts.len() >= #deeper {
                let #segment = __parts[#here];
                #inner
            }
        });
    }

    out
}

fn emit_leaf(
    leaf: &MatchLeaf,
    enum_ident: &Ident,
    bound: &[(String, usize)],
    link: &TokenStream,
) -> TokenStream {
    let variant = &leaf.variant;
    let fields = &leaf.fields;
    let mut build = quote! {
        return Some(#enum_ident::#variant { #(#fields),* });
    };

    for field in leaf.fields.iter().rev() {
        let depth = bound
            .iter()
            .find(|(name, _)| field.unraw() == name)
            .map(|(_, depth)| *depth)
            .unwrap_or_else(|| {
                panic!(
                    "routes!: `{}` has the field `{}` with no capture to fill it",
                    leaf.variant, field
                )
            });
        let segment = format_ident!("__seg_{}", depth);
        // The field's type comes from the struct expression below, so the
        // bound lands on the type the author declared without this crate ever
        // seeing it - and a type outside the set fails here, at the capture.
        build = quote! {
            if let Some(#field) = #link::LinkValue::decode(#segment) {
                #build
            }
        };
    }

    build
}

fn insert(root: &mut MatchNode, segments: &[Segment], leaf: &Leaf) {
    let mut node = root;
    for segment in segments {
        node = match segment {
            Segment::Literal(lit) => node.literal_mut(lit),
            Segment::Capture(name) => node.capture_mut(name),
        };
    }
    node.leaf = Some(MatchLeaf {
        ty: leaf.ty.clone(),
        variant: type_ident(&leaf.ty),
        fields: leaf.fields.iter().map(|field| field.name.clone()).collect(),
    });
}

fn shape_of(segments: &[Segment]) -> String {
    if segments.is_empty() {
        return "/".to_string();
    }
    let mut shape = String::new();
    for segment in segments {
        shape.push('/');
        match segment {
            Segment::Literal(lit) => shape.push_str(lit),
            Segment::Capture(_) => shape.push('*'),
        }
    }
    shape
}

fn captures_of(leaf: &Leaf) -> Vec<String> {
    let Some(link) = &leaf.link else {
        return Vec::new();
    };

    parse_pattern(link)
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Capture(name) => Some(name),
            Segment::Literal(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find_in_source;

    fn leaves_of(pages: &str) -> Vec<Leaf> {
        let source = format!("routes! {{ Route {{ {pages} }} }}");
        find_in_source(&source).expect("a route tree").leaves()
    }

    fn route_ident() -> Ident {
        format_ident!("Route")
    }

    /// Stands in for whatever `routes!` resolves the router crate to.
    fn link_mod() -> TokenStream {
        quote!(::link)
    }

    #[test]
    fn literals_are_tried_before_captures_however_they_were_declared() {
        let leaves = leaves_of(
            r#"
            page(Context) link("/:context/processes") { context: String }
            page(Settings) link("/settings/processes")
            "#,
        );
        let tree = build(&leaves).expect("no conflict");

        let children = tree.root().children();
        assert_eq!(
            children.iter().map(|(b, _)| b.clone()).collect::<Vec<_>>(),
            vec![
                Branch::Literal("settings".to_string()),
                Branch::Capture {
                    name: "context".to_string()
                },
            ]
        );
    }

    #[test]
    fn literals_keep_the_order_they_were_declared_in() {
        let leaves = leaves_of(
            r#"
            page(Beta) link("/beta")
            page(Alpha) link("/alpha")
            "#,
        );
        let tree = build(&leaves).expect("no conflict");

        assert_eq!(
            tree.root()
                .children()
                .iter()
                .map(|(b, _)| b.clone())
                .collect::<Vec<_>>(),
            vec![
                Branch::Literal("beta".to_string()),
                Branch::Literal("alpha".to_string()),
            ]
        );
    }

    #[test]
    fn a_shared_prefix_becomes_one_node() {
        let leaves = leaves_of(
            r#"
            page(Processes) link("/host/processes")
            page(Services) link("/host/services")
            "#,
        );
        let tree = build(&leaves).expect("no conflict");

        let children = tree.root().children();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].1.children().len(), 2);
        assert!(children[0].1.leaf().is_none());
    }

    #[test]
    fn a_page_sits_on_the_node_its_path_ends_at() {
        let leaves = leaves_of(r#"page(Processes) link("/host/processes")"#);
        let tree = build(&leaves).expect("no conflict");

        let root_children = tree.root().children();
        let host = root_children[0].1;
        let processes = host.children()[0].1;
        assert_eq!(processes.leaf().expect("a page").variant, "Processes");
    }

    #[test]
    fn two_pages_of_the_same_shape_conflict() {
        let leaves = leaves_of(
            r#"
            page(ByHost) link("/:host/processes") { host: String }
            page(ByPod) link("/:pod/processes") { pod: String }
            "#,
        );
        let conflicts = build(&leaves).expect_err("an ambiguity");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].shape, "/*/processes");
        assert_eq!(type_ident(&conflicts[0].first), "ByHost");
        assert_eq!(type_ident(&conflicts[0].second), "ByPod");
        assert!(conflicts[0].to_string().contains("/*/processes"));
    }

    #[test]
    fn a_literal_and_a_capture_in_the_same_place_are_not_a_conflict() {
        let leaves = leaves_of(
            r#"
            page(Settings) link("/settings/processes")
            page(Context) link("/:context/processes") { context: String }
            "#,
        );
        assert!(build(&leaves).is_ok());
    }

    #[test]
    fn captures_of_different_names_leading_elsewhere_are_separate_branches() {
        let leaves = leaves_of(
            r#"
            page(Processes) link("/:host/processes") { host: String }
            page(Pods) link("/:cluster/pods") { cluster: String }
            "#,
        );
        let tree = build(&leaves).expect("no conflict");

        assert_eq!(
            tree.root()
                .children()
                .iter()
                .map(|(b, _)| b.clone())
                .collect::<Vec<_>>(),
            vec![
                Branch::Capture {
                    name: "host".to_string()
                },
                Branch::Capture {
                    name: "cluster".to_string()
                },
            ]
        );
    }

    #[test]
    fn every_ambiguity_is_reported_at_once() {
        let leaves = leaves_of(
            r#"
            page(First) link("/:a") { a: String }
            page(Second) link("/:b") { b: String }
            page(Third) link("/:c") { c: String }
            "#,
        );
        let conflicts = build(&leaves).expect_err("two ambiguities");
        assert_eq!(conflicts.len(), 2);
    }

    #[test]
    fn a_declaration_whose_captures_and_fields_agree_has_no_field_errors() {
        let leaves = leaves_of(
            r#"
            page(Processes) link("/:context/processes") { context: String }
            page(Settings) link("/settings")
            "#,
        );
        assert!(check_fields(&leaves).is_empty());
    }

    #[test]
    fn a_capture_with_no_field_is_an_error() {
        let leaves = leaves_of(r#"page(Processes) link("/:context/processes")"#);
        let errors = check_fields(&leaves);

        assert_eq!(errors.len(), 1);
        let FieldError::CaptureWithoutField { page, capture } = &errors[0] else {
            panic!("expected a capture without a field, got {:?}", errors[0]);
        };
        assert_eq!(type_ident(page), "Processes");
        assert_eq!(capture, "context");
        assert!(errors[0].to_string().contains("context"));
    }

    #[test]
    fn a_field_with_no_capture_is_an_error() {
        let leaves = leaves_of(r#"page(Processes) link("/processes") { context: String }"#);
        let errors = check_fields(&leaves);

        assert_eq!(errors.len(), 1);
        let FieldError::FieldWithoutCapture { page, field } = &errors[0] else {
            panic!("expected a field without a capture, got {:?}", errors[0]);
        };
        assert_eq!(type_ident(page), "Processes");
        assert_eq!(field, "context");
    }

    #[test]
    fn a_capture_named_like_a_keyword_lands_in_its_raw_field() {
        let leaves = leaves_of(r#"page(Kind) link("/kind/:type") { r#type: String }"#);
        let errors = check_fields(&leaves);

        assert!(errors.is_empty(), "{:?}", errors.iter().map(ToString::to_string).collect::<Vec<_>>());

        let tree = build(&leaves).expect("no conflict");
        let generated = std::panic::catch_unwind(|| emit_parse(&tree, &route_ident(), &link_mod()).to_string());
        assert!(
            generated
                .as_deref()
                .is_ok_and(|generated| generated.contains("if let Some (r#type) = :: link :: LinkValue :: decode (__seg_1)")),
            "{generated:?}"
        );
    }

    #[test]
    fn a_capture_named_twice_is_an_error() {
        let leaves = leaves_of(r#"page(Pair) link("/pair/:id/:id") { id: String }"#);
        let errors = check_fields(&leaves);

        assert_eq!(errors.len(), 1, "{errors:?}");
        let text = errors[0].to_string();
        assert!(text.contains("`Pair` captures `:id` twice"), "{text}");
    }

    #[test]
    fn a_misspelt_field_is_reported_from_both_ends() {
        let leaves = leaves_of(r#"page(Processes) link("/:context/processes") { ctx: String }"#);
        assert_eq!(check_fields(&leaves).len(), 2);
    }

    #[test]
    fn one_route_generates_the_walk_down_to_it() {
        let leaves = leaves_of(r#"page(Processes) link("/:context/processes") { context: String }"#);
        let tree = build(&leaves).expect("no conflict");
        let generated = emit_parse(&tree, &route_ident(), &link_mod());

        let expected = quote! {
            pub fn parse(path: &str) -> Option<Self> {
                let __whole: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
                let __walk = |__parts: &[&str]| -> Option<Self> {
                    if __parts.len() >= 1 {
                        let __seg_0 = __parts[0];
                        if __parts.len() >= 2 && __parts[1] == "processes" {
                            if __parts.len() == 2 {
                                if let Some(context) = ::link::LinkValue::decode(__seg_0) {
                                    return Some(Route::Processes { context });
                                }
                            }
                        }
                    }
                    None
                };

                __walk(&__whole).or_else(|| match __whole.split_last() {
                    Some((&"", __rest)) => __walk(__rest),
                    _ => None,
                })
            }
        };

        assert_eq!(generated.to_string(), expected.to_string());
    }

    #[test]
    fn the_same_shape_conflicts_however_the_captures_would_parse() {
        let leaves = leaves_of(
            r#"
            page(Numbered) link("/:id/detail") { id: u32 }
            page(Named) link("/:name/detail") { name: String }
            "#,
        );
        let conflicts = build(&leaves).expect_err("the same shape twice");
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn the_generated_walk_never_leaves_parse_on_a_failed_capture() {
        let leaves = leaves_of(
            r#"
            page(Numbered) link("/count/:id") { id: u32 }
            page(Named) link("/count/all")
            "#,
        );
        let tree = build(&leaves).expect("no conflict");
        let generated = emit_parse(&tree, &route_ident(), &link_mod()).to_string();

        assert!(
            !generated.contains('?'),
            "a `?` would end the whole search, not the branch: {generated}"
        );
        assert!(generated.contains("if let Some (id) = :: link :: LinkValue :: decode (__seg_1)"));
    }

    #[test]
    fn a_branch_is_only_entered_when_the_path_is_long_enough() {
        let leaves = leaves_of(r#"page(Processes) link("/host/processes")"#);
        let tree = build(&leaves).expect("no conflict");
        let generated = emit_parse(&tree, &route_ident(), &link_mod()).to_string();

        assert!(generated.contains("__parts . len () >= 1 && __parts [0] == \"host\""));
        assert!(generated.contains("__parts . len () >= 2 && __parts [1] == \"processes\""));
        assert!(generated.contains("__parts . len () == 2"));
    }

    #[test]
    fn literals_are_generated_before_captures() {
        let leaves = leaves_of(
            r#"
            page(Context) link("/:context") { context: String }
            page(Settings) link("/settings")
            "#,
        );
        let tree = build(&leaves).expect("no conflict");
        let generated = emit_parse(&tree, &route_ident(), &link_mod()).to_string();

        let literal = generated.find("\"settings\"").expect("the literal branch");
        let capture = generated.find("__seg_0").expect("the capture branch");
        assert!(literal < capture, "{generated}");
    }

    #[test]
    fn the_root_can_hold_a_page_of_its_own() {
        let leaves = leaves_of(r#"page(Home) link("/")"#);
        let tree = build(&leaves).expect("no conflict");
        let generated = emit_parse(&tree, &route_ident(), &link_mod()).to_string();

        assert!(tree.root().leaf().is_some());
        assert!(generated.contains("__parts . len () == 0"));
        assert!(generated.contains("return Some (Route :: Home { })"));
    }

    #[test]
    fn a_declaration_with_no_pages_parses_nothing() {
        let tree = build(&[]).expect("no conflict");
        let generated = emit_parse(&tree, &route_ident(), &link_mod());

        let expected = quote! {
            pub fn parse(path: &str) -> Option<Self> {
                let _ = path;
                None
            }
        };
        assert_eq!(generated.to_string(), expected.to_string());
    }
}
