//! The route tree `routes!` declares, as data.
//!
//! Two things read the same declaration. `guinea-macros` expands it into an
//! enum, its chains and its URIs; a backend's build script generates from it
//! whatever that backend needs a route tree spelled out in - for Slint, the
//! `.slint` that holds every page and the branch each route shows.
//!
//! Both have to agree on what the declaration means, which is why the reading
//! of it lives here rather than in either.

use proc_macro2::{Delimiter, Ident, TokenStream, TokenTree};
use quote::quote;
use winnow::combinator::{alt, opt, preceded, separated};
use winnow::error::{ContextError, ErrMode};
use winnow::prelude::*;
use winnow::token::{any, take_till};

pub mod matcher;

/// A piece of a route pattern: `/:host/processes` is a capture and a literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Literal(String),
    Capture(String),
}

/// A node of the declared tree.
#[derive(Clone, Debug)]
pub enum Node {
    Layout {
        ty: syn::Type,
        guards: Guards,
        restorable: bool,
        children: Vec<Node>,
    },
    Page {
        ty: syn::Type,
        /// The address this page answers to, when it has agreed to have one.
        /// `None` is the default: a page reached only from inside, by value.
        link: Option<String>,
        guards: Guards,
        restorable: bool,
        fields: Vec<Field>,
    },
}

/// What a node said about the guards standing in front of it.
///
/// Two lists rather than one, because the two say opposite things and only one
/// of them cascades. `guard` tightens and is inherited by everything below;
/// `!guard` opens, applies where it is written, and has to name what it opens
/// so that removing protection reads as removing protection.
#[derive(Clone, Debug, Default)]
pub struct Guards {
    pub added: Vec<syn::Type>,
    pub removed: Vec<syn::Type>,
}

impl Guards {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// Folds this node's declarations into what it inherited.
    ///
    /// Removals first: a node that both drops an inherited guard and adds one
    /// of its own means both, and a node naming the same guard twice is saying
    /// something confused - which [`check_guards`] reports rather than
    /// silently resolving.
    ///
    /// Guards are told apart by their whole type as written: `HasRole<Admin>`
    /// and `HasRole<Guest>` are two guards, and so are `auth::Session` and
    /// `legacy::Session`.
    fn fold_into(&self, inherited: &mut Vec<syn::Type>) {
        inherited.retain(|standing| {
            !self
                .removed
                .iter()
                .any(|dropped| same_type(dropped, standing))
        });

        for added in &self.added {
            if !inherited.iter().any(|standing| same_type(standing, added)) {
                inherited.push(added.clone());
            }
        }
    }
}

/// One field of a route, and whether "the same" is a meaningful question
/// about it.
///
/// A route's fields are its parameters, and the router's one question about a
/// parameter is whether it is still the same one. For most fields that is
/// answerable. For a channel, a callback, an `Arc<dyn Trait>` it is not - they
/// have no identity, only an address - and `~` says so:
///
/// ```ignore
/// page(Wizard) { step: u8, result: ~Rc<Receiver<Report>> }
/// ```
///
/// The payload still reaches `install`; it is simply never compared and never
/// kept for the next comparison. Having thrown it away, the router cannot
/// claim two entries are the same, so a page carrying one reinstalls every
/// time - which is right, since a new channel is a new thing.
#[derive(Clone, Debug)]
pub struct Field {
    pub name: Ident,
    pub ty: syn::Type,
    /// `false` for a `~` field.
    pub identity: bool,
}

/// One page, with the layouts it sits inside - the chain the router installs.
#[derive(Clone, Debug)]
pub struct Leaf {
    pub ancestors: Vec<syn::Type>,
    pub ty: syn::Type,
    pub link: Option<String>,
    pub fields: Vec<Field>,
    /// What stands in front of this page, outermost first, with the cascade
    /// already folded in and the opt-outs already taken out. Resolved here
    /// because the tree is the only place that knows the path from the root.
    pub guards: Vec<syn::Type>,
    /// Whether this route survives a restart - declared here or inherited
    /// from a layout above.
    pub restorable: bool,
}

impl Leaf {
    /// Whether anything outside the application can name this page.
    pub fn is_addressable(&self) -> bool {
        self.link.is_some()
    }
}

/// A layout and the parameters it can count on.
///
/// Derived rather than declared: a layout may only rely on what *every* page
/// under it carries, so the list is the intersection of its descendants' -
/// name and type both. There is no second declaration to keep in sync, and a
/// page that stops carrying a field takes it out of its layouts by doing so.
///
/// `~` fields are never in it. A layout's parameters exist to be compared -
/// that comparison is what decides whether the layout survives a navigation -
/// and a field outside identity has nothing to contribute to it.
#[derive(Clone, Debug)]
pub struct LayoutParams {
    pub ty: syn::Type,
    pub fields: Vec<Field>,
}

/// A parsed `routes!` declaration.
#[derive(Clone, Debug)]
pub struct RouteTree {
    /// The enum the macro generates, and the type a route is.
    pub name: Ident,
    /// `backend = path::To::Backend`, when the declaration named one.
    pub backend: Option<syn::Type>,
    pub nodes: Vec<Node>,
}

impl RouteTree {
    /// Every page, in declaration order, with its ancestors.
    pub fn leaves(&self) -> Vec<Leaf> {
        let mut leaves = Vec::new();
        flatten(&self.nodes, &mut Vec::new(), &[], false, &mut leaves);
        leaves
    }

    /// The layouts in the tree, in declaration order, without repeats.
    pub fn layouts(&self) -> Vec<syn::Type> {
        let mut found = Vec::new();
        collect_layouts(&self.nodes, &mut found);
        found
    }

    /// Every layout with the parameters all of its pages carry.
    ///
    /// Keyed by type, not by position: `Layout::Params` is an associated type,
    /// so a layout appearing twice in the tree has one answer, and it is the
    /// intersection over both appearances.
    pub fn layout_params(&self) -> Vec<LayoutParams> {
        let leaves = self.leaves();

        self.layouts()
            .into_iter()
            .map(|ty| {
                let under: Vec<&Leaf> = leaves
                    .iter()
                    .filter(|leaf| leaf.ancestors.iter().any(|a| same_type(a, &ty)))
                    .collect();

                let fields = match under.split_first() {
                    Some((first, rest)) => first
                        .fields
                        .iter()
                        .filter(|field| field.identity)
                        .filter(|field| {
                            rest.iter().all(|leaf| {
                                leaf.fields.iter().any(|other| {
                                    other.identity
                                        && other.name == field.name
                                        && same_type(&field.ty, &other.ty)
                                })
                            })
                        })
                        .cloned()
                        .collect(),
                    // A layout with no pages under it is not reachable, so
                    // there is nothing it can be relied on to carry.
                    None => Vec::new(),
                };

                LayoutParams { ty, fields }
            })
            .collect()
    }
}

/// Types compared as written.
///
/// `syn::Type` has no cheap equality that means what is wanted here, and this
/// does: two fields are the same field when they are spelled the same way,
/// which is also the only thing the author can see.
pub fn same_type(left: &syn::Type, right: &syn::Type) -> bool {
    quote!(#left).to_string() == quote!(#right).to_string()
}

/// Parses the body of a `routes!` invocation.
///
/// Panics with the same messages the macro has always produced: this is the
/// macro's own reading, moved rather than rewritten.
pub fn parse(input: TokenStream) -> RouteTree {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut slice: Tokens = &tokens;

    let backend = parse_backend(&mut slice);

    let name = any_ident
        .parse_next(&mut slice)
        .unwrap_or_else(|_| panic!("routes! expects `EnumName {{ ... }}`"));
    let body_tokens = group_inner(Delimiter::Brace)
        .parse_next(&mut slice)
        .unwrap_or_else(|_| panic!("routes! expects a `{{ ... }}` body after `{name}`"));

    let mut body_slice: Tokens = &body_tokens;
    let nodes = parse_nodes(&mut body_slice).expect("failed to parse routes! body");

    RouteTree {
        name,
        backend,
        nodes,
    }
}

/// Finds the first `routes! { ... }` in a source file's tokens.
///
/// For build scripts, which have the file and not the expansion. Returns
/// `None` when the file declares no route tree.
pub fn find_in_source(source: &str) -> Option<RouteTree> {
    let file: TokenStream = source.parse().ok()?;
    find_invocation(file).map(parse)
}

fn find_invocation(stream: TokenStream) -> Option<TokenStream> {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();

    for (i, token) in tokens.iter().enumerate() {
        let TokenTree::Ident(name) = token else {
            continue;
        };
        if name != "routes" {
            continue;
        }
        match tokens.get(i + 1) {
            Some(TokenTree::Punct(p)) if p.as_char() == '!' => {}
            _ => continue,
        }
        if let Some(TokenTree::Group(body)) = tokens.get(i + 2) {
            return Some(body.stream());
        }
    }

    // Not at this level: a route tree may sit inside a module in the same file.
    tokens.into_iter().find_map(|token| match token {
        TokenTree::Group(group) => find_invocation(group.stream()),
        _ => None,
    })
}

/// The last segment of a page or layout's type path - `Processes` for
/// `pages::processes::Processes`.
pub fn type_ident(ty: &syn::Type) -> Ident {
    match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .expect("non-empty type path")
            .ident
            .clone(),
        _ => panic!("page(...)'s type must be a plain path (e.g. `Processes` or `pages::Processes`)"),
    }
}

/// Splits a route pattern into its segments.
pub fn parse_pattern(pattern: &str) -> Vec<Segment> {
    let trimmed = pattern.trim_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut input = trimmed;
    separated(1.., segment, '/')
        .parse_next(&mut input)
        .unwrap_or_else(|_| panic!("invalid route pattern {pattern:?}"))
}

fn flatten(
    nodes: &[Node],
    ancestors: &mut Vec<syn::Type>,
    standing: &[syn::Type],
    kept: bool,
    leaves: &mut Vec<Leaf>,
) {
    for node in nodes {
        match node {
            Node::Layout {
                ty,
                guards,
                restorable,
                children,
            } => {
                let mut inside = standing.to_vec();
                guards.fold_into(&mut inside);

                ancestors.push(ty.clone());
                flatten(children, ancestors, &inside, kept || *restorable, leaves);
                ancestors.pop();
            }
            Node::Page {
                ty,
                link,
                guards,
                restorable,
                fields,
            } => {
                let mut here = standing.to_vec();
                guards.fold_into(&mut here);

                leaves.push(Leaf {
                    ancestors: ancestors.clone(),
                    ty: ty.clone(),
                    link: link.clone(),
                    fields: fields.clone(),
                    guards: here,
                    restorable: kept || *restorable,
                });
            }
        }
    }
}

/// What the guard declarations say that cannot be true.
///
/// All of them, not the first: a tree whose guards drifted usually drifted in
/// several places, and one report per build turns one edit into several.
///
/// The cascade is allowed to be implicit precisely because undoing it is not,
/// so an opt-out that opens nothing is the mistake worth catching. It reads
/// exactly like protection being removed, and removes nothing.
pub fn check_guards(tree: &RouteTree) -> Vec<String> {
    let mut errors = Vec::new();
    walk_guards(&tree.nodes, &[], &mut errors);
    errors
}

fn walk_guards(nodes: &[Node], standing: &[syn::Type], errors: &mut Vec<String>) {
    for node in nodes {
        let (ty, guards, children) = match node {
            Node::Layout {
                ty,
                guards,
                children,
                ..
            } => (ty, guards, Some(children)),
            Node::Page { ty, guards, .. } => (ty, guards, None),
        };

        let here = spelled(ty);
        let is_standing = |wanted: &syn::Type, list: &[syn::Type]| {
            list.iter().any(|seen| same_type(seen, wanted))
        };

        for dropped in &guards.removed {
            if !is_standing(dropped, standing) {
                errors.push(format!(
                    "routes!: `!guard({})` on `{here}` opens nothing - no layout above it \
                     declared that guard",
                    spelled(dropped)
                ));
            }
        }

        let mut inside = standing.to_vec();
        guards.fold_into(&mut inside);

        for (at, added) in guards.added.iter().enumerate() {
            if is_standing(added, standing) && !is_standing(added, &guards.removed) {
                errors.push(format!(
                    "routes!: `guard({})` on `{here}` is already standing from above",
                    spelled(added)
                ));
            }
            if guards.added[..at]
                .iter()
                .any(|earlier| same_type(earlier, added))
            {
                errors.push(format!(
                    "routes!: `guard({})` is declared twice on `{here}`",
                    spelled(added)
                ));
            }
        }

        if let Some(children) = children {
            walk_guards(children, &inside, errors);
        }
    }
}

/// Layouts placed more than once under different layouts.
///
/// A layout is one type with one answer to what stands above it - what it
/// may read from its ancestors is checked against that answer when it
/// compiles. Placed again under other ancestors, it would compile against
/// the first place and find nothing to read in the second.
pub fn check_layouts(tree: &RouteTree) -> Vec<String> {
    let mut placed: Vec<(syn::Type, Vec<syn::Type>)> = Vec::new();
    place_layouts(&tree.nodes, &mut Vec::new(), &mut placed);

    let mut errors = Vec::new();
    for (at, (ty, above)) in placed.iter().enumerate() {
        let first = placed[..at]
            .iter()
            .find(|(earlier, _)| same_type(earlier, ty));
        let Some((_, before)) = first else {
            continue;
        };

        let same_above = before.len() == above.len()
            && before.iter().zip(above).all(|(a, b)| same_type(a, b));
        if !same_above {
            let path = |above: &[syn::Type]| match above {
                [] => "the root".to_string(),
                _ => above.iter().map(spelled).collect::<Vec<_>>().join(" > "),
            };
            errors.push(format!(
                "routes!: `layout({})` stands under {} in one place and under {} in another - \
                 a layout is placed under the same layouts wherever it appears",
                spelled(ty),
                path(before),
                path(above)
            ));
        }
    }
    errors
}

fn place_layouts(
    nodes: &[Node],
    above: &mut Vec<syn::Type>,
    placed: &mut Vec<(syn::Type, Vec<syn::Type>)>,
) {
    for node in nodes {
        if let Node::Layout { ty, children, .. } = node {
            placed.push((ty.clone(), above.clone()));

            above.push(ty.clone());
            place_layouts(children, above, placed);
            above.pop();
        }
    }
}

fn collect_layouts(nodes: &[Node], found: &mut Vec<syn::Type>) {
    for node in nodes {
        if let Node::Layout { ty, children, .. } = node {
            if !found.iter().any(|seen| same_type(seen, ty)) {
                found.push(ty.clone());
            }
            collect_layouts(children, found);
        }
    }
}

type Tokens<'i> = &'i [TokenTree];

fn fail<O>() -> ModalResult<O> {
    Err(ErrMode::Backtrack(ContextError::new()))
}

fn kw<'i>(word: &'static str) -> impl Parser<Tokens<'i>, (), ErrMode<ContextError>> {
    move |input: &mut Tokens<'i>| match any.parse_next(input)? {
        TokenTree::Ident(id) if id == word => Ok(()),
        _ => fail(),
    }
}

fn any_ident<'i>(input: &mut Tokens<'i>) -> ModalResult<Ident> {
    match any.parse_next(input)? {
        TokenTree::Ident(id) => Ok(id),
        _ => fail(),
    }
}

fn punct<'i>(ch: char) -> impl Parser<Tokens<'i>, (), ErrMode<ContextError>> {
    move |input: &mut Tokens<'i>| match any.parse_next(input)? {
        TokenTree::Punct(p) if p.as_char() == ch => Ok(()),
        _ => fail(),
    }
}

fn group_inner<'i>(
    delim: Delimiter,
) -> impl Parser<Tokens<'i>, Vec<TokenTree>, ErrMode<ContextError>> {
    move |input: &mut Tokens<'i>| match any.parse_next(input)? {
        TokenTree::Group(g) if g.delimiter() == delim => Ok(g.stream().into_iter().collect()),
        _ => fail(),
    }
}

fn string_lit<'i>(input: &mut Tokens<'i>) -> ModalResult<String> {
    match any.parse_next(input)? {
        TokenTree::Literal(lit) => {
            let s: syn::LitStr =
                syn::parse2(quote!(#lit)).map_err(|_| ErrMode::Backtrack(ContextError::new()))?;
            Ok(s.value())
        }
        _ => fail(),
    }
}

fn parse_type<'i>(
    stop: impl Fn(&TokenTree) -> bool,
) -> impl Parser<Tokens<'i>, syn::Type, ErrMode<ContextError>> {
    move |input: &mut Tokens<'i>| {
        let mut collected = Vec::new();
        while let Some(tt) = input.first() {
            if stop(tt) {
                break;
            }
            collected.push(any.parse_next(input)?);
        }
        if collected.is_empty() {
            return fail();
        }
        let ts: TokenStream = collected.into_iter().collect();
        syn::parse2::<syn::Type>(ts).map_err(|_| ErrMode::Backtrack(ContextError::new()))
    }
}

/// The one type a pair of parentheses holds - `page(..)`, `layout(..)`,
/// `guard(..)`.
///
/// Read by syn, so `HasRole<Admin, Read>` is one type and not two halves of a
/// list. Anything after it is a mistake the author should hear about rather
/// than have dropped: `refused` says what the mistake was.
fn sole_type(paren: Vec<TokenTree>, refused: impl FnOnce(&syn::Type) -> String) -> ModalResult<syn::Type> {
    use syn::parse::{ParseStream, Parser};

    let stream: TokenStream = paren.into_iter().collect();
    let read = |input: ParseStream| {
        let ty: syn::Type = input.parse()?;
        let rest: TokenStream = input.parse()?;
        Ok((ty, rest))
    };

    match read.parse2(stream) {
        Ok((ty, rest)) if rest.is_empty() => Ok(ty),
        Ok((ty, _)) => panic!("{}", refused(&ty)),
        Err(_) => fail(),
    }
}

/// A type as the author wrote it, for messages and names.
pub fn spelled(ty: &syn::Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
}

fn is_comma(tt: &TokenTree) -> bool {
    matches!(tt, TokenTree::Punct(p) if p.as_char() == ',')
}

fn parse_fields(tokens: Vec<TokenTree>) -> Vec<Field> {
    let mut slice: Tokens = &tokens;
    let mut fields = Vec::new();
    while !slice.is_empty() {
        let name = any_ident
            .parse_next(&mut slice)
            .expect("field name in page(...) { ... }");
        punct(':')
            .parse_next(&mut slice)
            .expect("`:` after field name");

        // `~` before the type, reading as "approximately": a field for which
        // "the same" is not a meaningful question. The token is vacant in Rust
        // - it meant `~T` before 1.0 and was never reused - and it lexes
        // cleanly here.
        let identity = opt(punct('~'))
            .parse_next(&mut slice)
            .expect("`opt` does not fail")
            .is_none();

        let ty = parse_type(is_comma)
            .parse_next(&mut slice)
            .expect("field type in page(...) { ... }");
        fields.push(Field { name, ty, identity });
        let _ = punct(',').parse_next(&mut slice);
    }
    fields
}

fn parse_layout_node<'i>(input: &mut Tokens<'i>) -> ModalResult<Node> {
    kw("layout").parse_next(input)?;
    let paren_tokens = group_inner(Delimiter::Parenthesis).parse_next(input)?;
    let ty = sole_type(paren_tokens, |ty| {
        format!("routes!: `layout({}, ..)` - a layout is one type", spelled(ty))
    })?;

    let mut guards = Guards::default();
    let mut restorable = false;
    loop {
        let declared = parse_guards(input);
        if !declared.is_empty() {
            guards.added.extend(declared.added);
            guards.removed.extend(declared.removed);
            continue;
        }
        if parse_restorable(input) {
            restorable = true;
            continue;
        }
        break;
    }

    let brace_tokens = group_inner(Delimiter::Brace).parse_next(input)?;
    let mut brace_slice: Tokens = &brace_tokens;
    let children = parse_nodes(&mut brace_slice)?;
    Ok(Node::Layout {
        ty,
        guards,
        restorable,
        children,
    })
}

fn parse_page_node<'i>(input: &mut Tokens<'i>) -> ModalResult<Node> {
    kw("page").parse_next(input)?;
    let paren_tokens = group_inner(Delimiter::Parenthesis).parse_next(input)?;
    let ty = sole_type(paren_tokens, |ty| {
        format!(
            "routes!: `page({0}, ..)` - a page is one type, and its address is written \
             after it: `page({0}) link(\"/..\")`",
            spelled(ty)
        )
    })?;

    // Modifiers in any order, so `link(..) guard(..)` and `guard(..) link(..)`
    // both read - there is no reason for one to come first and nothing to gain
    // by making the author remember which.
    let mut link = None;
    let mut guards = Guards::default();
    let mut restorable = false;
    loop {
        if let Some(found) = parse_link(input) {
            link = Some(found);
            continue;
        }
        let declared = parse_guards(input);
        if !declared.is_empty() {
            guards.added.extend(declared.added);
            guards.removed.extend(declared.removed);
            continue;
        }
        if parse_restorable(input) {
            restorable = true;
            continue;
        }
        break;
    }

    let fields = match input.first() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            any.parse_next(input)?;
            parse_fields(inner)
        }
        _ => Vec::new(),
    };

    Ok(Node::Page {
        ty,
        link,
        guards,
        restorable,
        fields,
    })
}

/// `restorable`, if it is there.
///
/// A word on its own rather than a call: it takes no argument, and the tier it
/// opts into is the whole statement. It cascades, because it tightens - and it
/// has no negation, unlike `guard`. Opting out would make a layout's claim
/// false: an area that says it survives a restart, with a page inside it that
/// does not, restores into nothing, and implicit onward resolution is what
/// breaks history everywhere it is tried. A page that cannot be restored does
/// not belong under a layout that can.
fn parse_restorable<'i>(input: &mut Tokens<'i>) -> bool {
    match input.first() {
        Some(TokenTree::Ident(word)) if word == "restorable" => {
            *input = &input[1..];
            true
        }
        _ => false,
    }
}

/// `guard(RequiresAdmin)` or `!guard(RequiresAdmin)`, if either is there.
///
/// The negation has to name what it opens. A bare `!guard` would let a page
/// shed protection without saying which, and the whole reason the cascade is
/// allowed to be implicit is that undoing it is not.
fn parse_guards<'i>(input: &mut Tokens<'i>) -> Guards {
    let mut guards = Guards::default();

    loop {
        let (removing, at) = match input.first() {
            Some(TokenTree::Punct(p)) if p.as_char() == '!' => (true, 1),
            _ => (false, 0),
        };

        match input.get(at) {
            Some(TokenTree::Ident(word)) if word == "guard" => {}
            _ => return guards,
        }

        let mut slice: Tokens = &input[at + 1..];
        let Ok(paren) = group_inner(Delimiter::Parenthesis).parse_next(&mut slice) else {
            return guards;
        };

        let sign = if removing { "!" } else { "" };
        let Ok(ty) = sole_type(paren, |ty| {
            format!(
                "routes!: `{sign}guard({0}, ..)` - one guard per `{sign}guard(..)`: write \
                 `{sign}guard({0}) {sign}guard(..)`",
                spelled(ty)
            )
        }) else {
            return guards;
        };

        if removing {
            guards.removed.push(ty);
        } else {
            guards.added.push(ty);
        }
        *input = &input[at + 2..];
    }
}

/// `link("/m/:context")`, if it is there. Modifiers are read before the body,
/// so a brace ends the node and nothing has to look ahead.
fn parse_link<'i>(input: &mut Tokens<'i>) -> Option<String> {
    let Some(TokenTree::Ident(word)) = input.first() else {
        return None;
    };
    if word != "link" {
        return None;
    }

    let mut slice: Tokens = &input[1..];
    let paren = group_inner(Delimiter::Parenthesis).parse_next(&mut slice).ok()?;
    let mut inner: Tokens = &paren;
    let literal = string_lit.parse_next(&mut inner).ok()?;
    if !inner.is_empty() {
        panic!("routes!: `link({literal:?}, ..)` - a page answers to one address");
    }

    *input = &input[2..];
    Some(literal)
}

fn parse_node<'i>(input: &mut Tokens<'i>) -> ModalResult<Node> {
    alt((parse_layout_node, parse_page_node)).parse_next(input)
}

fn parse_nodes<'i>(input: &mut Tokens<'i>) -> ModalResult<Vec<Node>> {
    let mut nodes = Vec::new();
    while !input.is_empty() {
        nodes.push(parse_node.parse_next(input)?);
    }
    Ok(nodes)
}

fn segment<'i>(input: &mut &'i str) -> ModalResult<Segment> {
    alt((
        preceded(':', take_till(0.., '/')).map(|s: &str| Segment::Capture(s.to_string())),
        take_till(1.., '/').map(|s: &str| Segment::Literal(s.to_string())),
    ))
    .parse_next(input)
}

/// The backend a route tree mounts on.
///
/// Written as one type - `backend = guinea_ratatui::Tui` - because that is
/// what an application knows.
fn parse_backend(slice: &mut Tokens) -> Option<syn::Type> {
    let tokens = *slice;
    match tokens.first() {
        Some(TokenTree::Ident(id)) if id == "backend" => {}
        _ => return None,
    }
    match tokens.get(1) {
        Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
        _ => panic!("routes! expects `backend = path::To::Backend,`"),
    }

    let mut end = 2;
    while end < tokens.len() {
        match &tokens[end] {
            TokenTree::Punct(p) if p.as_char() == ',' => break,
            _ => end += 1,
        }
    }
    if end == tokens.len() {
        panic!("routes! expects `,` after the backend");
    }

    let ty: syn::Type = syn::parse2(tokens[2..end].iter().cloned().collect())
        .unwrap_or_else(|e| panic!("routes!: backend is not a type: {e}"));

    *slice = &tokens[end + 1..];
    Some(ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECLARATION: &str = r#"
        routes! {
            backend = guinea::slint::Slint,
            Route {
                layout(TabsLayout) {
                    page(Processes) link("/:context/processes") { context: String }
                    page(Services) link("/:context/services") { context: String }
                }
            }
        }
    "#;

    #[test]
    fn a_declaration_flattens_to_one_leaf_per_page() {
        let tree = find_in_source(DECLARATION).expect("a route tree");
        let leaves = tree.leaves();

        assert_eq!(tree.name, "Route");
        assert_eq!(leaves.len(), 2);
        assert_eq!(type_ident(&leaves[0].ty), "Processes");
        assert_eq!(
            leaves[0].ancestors.iter().map(type_ident).collect::<Vec<_>>(),
            vec!["TabsLayout"],
            "a page carries the layouts it sits inside"
        );
    }

    #[test]
    fn a_layout_wrapping_several_pages_is_listed_once() {
        let tree = find_in_source(DECLARATION).expect("a route tree");
        assert_eq!(
            tree.layouts().iter().map(type_ident).collect::<Vec<_>>(),
            vec!["TabsLayout"]
        );
    }

    fn params_of(tree: &RouteTree, layout: &str) -> Vec<String> {
        tree.layout_params()
            .into_iter()
            .find(|found| type_ident(&found.ty) == layout)
            .unwrap_or_else(|| panic!("no layout named {layout}"))
            .fields
            .iter()
            .map(|field| {
                let (name, ty) = (&field.name, &field.ty);
                format!("{name}: {}", quote!(#ty))
            })
            .collect()
    }

    #[test]
    fn a_layout_carries_what_every_page_under_it_carries() {
        let tree = find_in_source(DECLARATION).expect("a route tree");
        assert_eq!(params_of(&tree, "TabsLayout"), ["context: String"]);
    }

    #[test]
    fn a_field_only_some_pages_carry_is_not_the_layouts_to_rely_on() {
        let tree = find_in_source(
            r#"
            routes! {
                Route {
                    layout(TabsLayout) {
                        page(Processes) { context: String, pid: u32 }
                        page(Services) { context: String }
                    }
                }
            }
            "#,
        )
        .expect("a route tree");

        assert_eq!(
            params_of(&tree, "TabsLayout"),
            ["context: String"],
            "`pid` reaches only one of the two, so the layout cannot be handed it"
        );
    }

    #[test]
    fn a_name_shared_with_a_different_type_is_not_shared_at_all() {
        let tree = find_in_source(
            r#"
            routes! {
                Route {
                    layout(TabsLayout) {
                        page(Processes) { context: String }
                        page(Services) { context: u32 }
                    }
                }
            }
            "#,
        )
        .expect("a route tree");

        assert!(
            params_of(&tree, "TabsLayout").is_empty(),
            "two fields named the same are one field only if they are the same type"
        );
    }

    #[test]
    fn an_inner_layout_may_rely_on_more_than_the_outer_one() {
        let tree = find_in_source(
            r#"
            routes! {
                Route {
                    layout(Shell) {
                        layout(TabsLayout) {
                            page(Processes) { context: String, tab: u8 }
                            page(Services) { context: String, tab: u8 }
                        }
                        page(Splash) { context: String }
                    }
                }
            }
            "#,
        )
        .expect("a route tree");

        assert_eq!(params_of(&tree, "Shell"), ["context: String"]);
        assert_eq!(
            params_of(&tree, "TabsLayout"),
            ["context: String", "tab: u8"],
            "the intersection is over its own descendants, not the whole tree"
        );
    }

    #[test]
    fn patterns_split_into_literals_and_captures() {
        assert_eq!(
            parse_pattern("/:context/processes"),
            vec![
                Segment::Capture("context".to_string()),
                Segment::Literal("processes".to_string()),
            ]
        );
    }

    #[test]
    fn a_route_tree_is_found_inside_a_module() {
        let source = format!("mod routes {{ use super::*; {DECLARATION} }}");
        assert!(find_in_source(&source).is_some());
    }

    #[test]
    fn a_file_without_one_is_not_an_error() {
        assert!(find_in_source("fn main() {}").is_none());
    }

    fn tree_of(body: &str) -> RouteTree {
        find_in_source(&format!("routes! {{ Route {{ {body} }} }}")).expect("a route tree")
    }

    fn guards_of(tree: &RouteTree, page: &str) -> Vec<String> {
        tree.leaves()
            .into_iter()
            .find(|leaf| type_ident(&leaf.ty) == page)
            .expect("the page")
            .guards
            .iter()
            .map(spelled)
            .collect()
    }

    #[test]
    fn a_guard_cascades_to_everything_under_it() {
        let tree = tree_of(
            r#"
            layout(AdminArea) guard(RequiresAdmin) {
                page(Audit)
                layout(Inner) guard(Twice) {
                    page(Deep)
                }
            }
            page(Splash)
            "#,
        );

        assert_eq!(guards_of(&tree, "Audit"), ["RequiresAdmin"]);
        assert_eq!(
            guards_of(&tree, "Deep"),
            ["RequiresAdmin", "Twice"],
            "outermost first, which is the order they are asked in"
        );
        assert_eq!(
            guards_of(&tree, "Splash"),
            [] as [&str; 0],
            "a sibling of the area is not inside it"
        );
    }

    #[test]
    fn opting_out_takes_one_guard_and_leaves_the_rest() {
        let tree = tree_of(
            r#"
            layout(Outer) guard(A) {
                layout(Inner) guard(B) {
                    page(Both)
                    page(OnlyA) !guard(B)
                    page(OnlyB) !guard(A)
                }
            }
            "#,
        );

        assert_eq!(guards_of(&tree, "Both"), ["A", "B"]);
        assert_eq!(guards_of(&tree, "OnlyA"), ["A"]);
        assert_eq!(guards_of(&tree, "OnlyB"), ["B"]);
    }

    #[test]
    fn an_opt_out_that_opens_nothing_is_reported() {
        // It reads exactly like protection being removed, and removes nothing.
        let tree = tree_of(r#"page(Splash) !guard(NeverDeclared)"#);
        let errors = check_guards(&tree);

        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("NeverDeclared"));
        assert!(errors[0].contains("opens nothing"));
    }

    #[test]
    fn a_guard_declared_twice_over_is_reported() {
        let tree = tree_of(
            r#"
            layout(Area) guard(A) {
                page(Again) guard(A)
            }
            "#,
        );

        let errors = check_guards(&tree);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("already standing"));
    }

    #[test]
    fn dropping_a_guard_and_declaring_it_again_is_allowed() {
        // Not a contradiction: it says "not the one from above, this one" -
        // which is the only way to narrow an inherited guard.
        let tree = tree_of(
            r#"
            layout(Area) guard(A) {
                page(Narrower) !guard(A) guard(A)
            }
            "#,
        );

        assert!(check_guards(&tree).is_empty());
        assert_eq!(guards_of(&tree, "Narrower"), ["A"]);
    }

    fn restorable_of(tree: &RouteTree, page: &str) -> bool {
        tree.leaves()
            .into_iter()
            .find(|leaf| type_ident(&leaf.ty) == page)
            .expect("the page")
            .restorable
    }

    #[test]
    fn restorable_cascades_and_has_no_way_out() {
        let tree = tree_of(
            r#"
            layout(Session) restorable {
                page(Inside)
                layout(Deeper) {
                    page(Further)
                }
            }
            page(Outside)
            page(OnItsOwn) restorable
            "#,
        );

        assert!(restorable_of(&tree, "Inside"));
        assert!(restorable_of(&tree, "Further"), "through a plain layout too");
        assert!(!restorable_of(&tree, "Outside"));
        assert!(restorable_of(&tree, "OnItsOwn"), "a page may claim it alone");
    }

    #[test]
    fn a_payload_is_declared_by_the_field_and_nothing_else() {
        let tree = tree_of(r#"page(Wizard) { step: u8, result: ~Receiver<Report> }"#);
        let fields = &tree.leaves()[0].fields;

        assert_eq!(fields.len(), 2);
        assert!(fields[0].identity, "step is what makes it the same wizard");
        assert!(!fields[1].identity);
        let payload = &fields[1].ty;
        assert_eq!(
            quote!(#payload).to_string(),
            quote!(Receiver<Report>).to_string(),
            "the `~` is not part of the type"
        );
    }

    #[test]
    fn a_payload_on_a_route_that_reaches_outside_is_reported() {
        for declaration in [
            r#"page(Wizard) link("/wizard/:step") { step: u8, result: ~Feed }"#,
            r#"page(Wizard) restorable { step: u8, result: ~Feed }"#,
        ] {
            let tree = tree_of(declaration);
            let errors = matcher::check_fields(&tree.leaves());

            assert!(
                errors.iter().any(|error| {
                    let text = error.to_string();
                    text.contains("result") && text.contains("never kept")
                }),
                "{declaration} should be refused: {errors:?}"
            );
        }
    }

    #[test]
    fn modifiers_read_in_either_order() {
        let tree = tree_of(r#"page(Audit) guard(RequiresAdmin) restorable link("/audit")"#);
        let other = tree_of(r#"page(Audit) link("/audit") restorable guard(RequiresAdmin)"#);

        for tree in [&tree, &other] {
            let leaf = &tree.leaves()[0];
            assert_eq!(leaf.link.as_deref(), Some("/audit"));
            assert_eq!(guards_of(tree, "Audit"), ["RequiresAdmin"]);
            assert!(restorable_of(tree, "Audit"));
        }
    }

    #[test]
    #[should_panic(expected = "one guard per `guard(..)`")]
    fn two_guards_in_one_pair_of_parentheses_are_refused() {
        tree_of(r#"layout(Admin) guard(RequiresSession, RequiresAdmin) { page(Users) }"#);
    }

    #[test]
    #[should_panic(expected = "one guard per `!guard(..)`")]
    fn two_opt_outs_in_one_pair_of_parentheses_are_refused() {
        tree_of(r#"layout(Admin) guard(A) guard(B) { page(Users) !guard(A, B) }"#);
    }

    #[test]
    #[should_panic(expected = r#"`page(Processes) link("/..")`"#)]
    fn an_address_inside_a_pages_parentheses_is_refused() {
        tree_of(r#"page(Processes, "/:context/processes") { context: String }"#);
    }

    #[test]
    #[should_panic(expected = "a page answers to one address")]
    fn two_addresses_are_refused() {
        tree_of(r#"page(Processes) link("/a", "/b")"#);
    }

    #[test]
    fn a_generic_guard_is_one_type() {
        let tree = tree_of(r#"page(Audit) guard(HasRole<Admin, Read>)"#);
        assert_eq!(guards_of(&tree, "Audit"), ["HasRole<Admin,Read>"]);
    }

    #[test]
    fn guards_are_told_apart_by_their_whole_type() {
        let tree = tree_of(
            r#"
            layout(AdminArea) guard(HasRole<Admin>) {
                page(Help) !guard(HasRole<Guest>)
                page(Users) guard(HasRole<Guest>)
            }
            layout(Other) guard(auth::Session) {
                page(Legacy) guard(legacy::Session)
            }
            "#,
        );

        assert_eq!(guards_of(&tree, "Help"), ["HasRole<Admin>"]);
        assert_eq!(guards_of(&tree, "Users"), ["HasRole<Admin>", "HasRole<Guest>"]);
        assert_eq!(guards_of(&tree, "Legacy"), ["auth::Session", "legacy::Session"]);

        let errors = check_guards(&tree);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("`!guard(HasRole<Guest>)` on `Help` opens nothing"));
    }

    #[test]
    fn a_layout_under_other_layouts_elsewhere_is_refused() {
        let tree = tree_of(
            r#"
            layout(Shell) { layout(Tabs) { page(A) } }
            layout(Tabs) { page(B) }
            "#,
        );

        let errors = check_layouts(&tree);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("under Shell in one place and under the root in another"));
    }

    #[test]
    fn a_layout_under_the_same_layouts_twice_is_fine() {
        let tree = tree_of(
            r#"
            layout(Shell) {
                layout(Tabs) { page(A) }
                page(Between)
                layout(Tabs) { page(B) }
            }
            "#,
        );

        assert!(check_layouts(&tree).is_empty());
    }
}
