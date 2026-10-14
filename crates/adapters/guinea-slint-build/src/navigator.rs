//! The `Navigator` global and its Rust side, written from the `Route` enum.

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::anyhow;
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::{Fields, Ident, Item, ItemEnum, Path, Token, Type};

/// What [`emit`] writes: the `.slint` the window imports, and the Rust the
/// application includes beside its route.
#[derive(Debug)]
pub(crate) struct Generated {
    pub slint: String,
    pub rust: String,
}

/// A place the window can be: the variants from the root enum down to it,
/// each with the enum it belongs to, and its parameters.
struct Leaf {
    path: Vec<(Ident, Ident)>,
    fields: Vec<Field>,
}

struct Field {
    name: Ident,
    slint: &'static str,
}

/// Reads the `Route` enum in `source`, and the route enums it nests, and
/// writes the global that shows it. `adapter` is the path generated Rust
/// reaches `guinea-slint` by.
pub(crate) fn emit(source: &str, adapter: &str) -> anyhow::Result<Generated> {
    let file = syn::parse_file(source)?;
    let routes: HashMap<String, &ItemEnum> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(declared) if derives_route(declared) => Some((declared.ident.to_string(), declared)),
            _ => None,
        })
        .collect();

    let root = routes
        .get("Route")
        .ok_or_else(|| anyhow!("no `#[derive(Route)] enum Route` in it"))?;

    let mut leaves = Vec::new();
    collect(root, &routes, &[], &mut leaves)?;
    refuse_same_names(&leaves)?;

    Ok(Generated {
        slint: slint(&leaves),
        rust: rust(&leaves, adapter),
    })
}

fn derives_route(declared: &ItemEnum) -> bool {
    declared.attrs.iter().any(|attr| {
        attr.path().is_ident("derive")
            && attr
                .parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated)
                .is_ok_and(|derived| {
                    derived
                        .iter()
                        .any(|path| path.segments.last().is_some_and(|last| last.ident == "Route"))
                })
    })
}

fn collect(
    declared: &ItemEnum,
    routes: &HashMap<String, &ItemEnum>,
    above: &[(Ident, Ident)],
    into: &mut Vec<Leaf>,
) -> anyhow::Result<()> {
    for variant in &declared.variants {
        let mut path = above.to_vec();
        path.push((declared.ident.clone(), variant.ident.clone()));

        match &variant.fields {
            Fields::Unit => into.push(Leaf { path, fields: Vec::new() }),
            Fields::Named(named) => {
                let mut fields = Vec::new();
                for field in &named.named {
                    let name = field.ident.clone().expect("a named field");
                    let slint = slint_type(&field.ty).ok_or_else(|| {
                        anyhow!(
                            "`{}.{name}` is a `{}`, which the Navigator global cannot carry: \
                             a parameter is a String, a bool, an integer or a float",
                            shown(&path),
                            field.ty.to_token_stream()
                        )
                    })?;
                    fields.push(Field { name, slint });
                }
                into.push(Leaf { path, fields });
            }
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let nested = last_ident(&unnamed.unnamed[0].ty);
                let found = nested.as_ref().and_then(|nested| routes.get(&nested.to_string()));
                let Some(found) = found else {
                    return Err(anyhow!(
                        "`{}` nests `{}`, which is not a `#[derive(Route)]` enum in this file",
                        shown(&path),
                        unnamed.unnamed[0].ty.to_token_stream()
                    ));
                };
                collect(found, routes, &path, into)?;
            }
            Fields::Unnamed(_) => {
                return Err(anyhow!(
                    "`{}` holds more than one value: a tuple variant holds the route of the level below",
                    shown(&path)
                ));
            }
        }
    }
    Ok(())
}

fn last_ident(ty: &Type) -> Option<Ident> {
    match ty {
        Type::Path(path) if path.qself.is_none() => {
            let last = path.path.segments.last()?;
            last.arguments.is_none().then(|| last.ident.clone())
        }
        _ => None,
    }
}

fn slint_type(ty: &Type) -> Option<&'static str> {
    let ident = last_ident(ty)?.to_string();
    match ident.as_str() {
        "String" => Some("string"),
        "bool" => Some("bool"),
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => Some("int"),
        "f32" | "f64" => Some("float"),
        _ => None,
    }
}

/// `Route::Processes::One`: the place as the error messages name it.
fn shown(path: &[(Ident, Ident)]) -> String {
    let mut said = path.first().map(|(root, _)| root.to_string()).unwrap_or_default();
    for (_, variant) in path {
        let _ = write!(said, "::{variant}");
    }
    said
}

fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (at, character) in name.chars().enumerate() {
        if character.is_uppercase() {
            if at > 0 {
                out.push('-');
            }
            out.extend(character.to_lowercase());
        } else if character == '_' {
            out.push('-');
        } else {
            out.push(character);
        }
    }
    out
}

impl Leaf {
    /// `ProcessesOne`: the variants down to it, as the Rust side of `RouteId`
    /// names it.
    fn id(&self) -> String {
        self.path.iter().map(|(_, variant)| variant.to_string()).collect()
    }

    /// `processes-one`: the same, as Slint names it.
    fn kebab(&self) -> String {
        kebab(&self.id())
    }

    fn params(&self) -> String {
        format!("{}Params", self.id())
    }

    /// The place itself, its innermost variant given `fields`.
    fn nested(&self, fields: &str) -> String {
        let mut built = String::new();
        for (at, (declared, variant)) in self.path.iter().enumerate() {
            let _ = write!(built, "{declared}::{variant}");
            if at + 1 < self.path.len() {
                built.push('(');
            }
        }
        built.push_str(fields);
        for _ in 1..self.path.len() {
            built.push(')');
        }
        built
    }

    fn pattern(&self) -> String {
        match self.fields.is_empty() {
            true => self.nested(""),
            false => {
                let names: Vec<String> = self.fields.iter().map(|field| field.name.to_string()).collect();
                self.nested(&format!(" {{ {} }}", names.join(", ")))
            }
        }
    }

    fn built_from_params(&self, adapter: &str) -> String {
        match self.fields.is_empty() {
            true => self.nested(""),
            false => {
                let fields: Vec<String> = self
                    .fields
                    .iter()
                    .map(|field| format!("{0}: {adapter}::FromSlint::from_slint(params.{0})", field.name))
                    .collect();
                self.nested(&format!(" {{ {} }}", fields.join(", ")))
            }
        }
    }
}

fn refuse_same_names(leaves: &[Leaf]) -> anyhow::Result<()> {
    let mut seen: HashMap<String, &Leaf> = HashMap::new();
    for leaf in leaves {
        if let Some(first) = seen.insert(leaf.kebab(), leaf) {
            return Err(anyhow!(
                "`{}` and `{}` are both `RouteId.{}` in Slint: rename one",
                shown(&first.path),
                shown(&leaf.path),
                leaf.kebab()
            ));
        }
    }
    Ok(())
}

fn slint(leaves: &[Leaf]) -> String {
    let mut out = String::from("export enum RouteId {\n");
    for leaf in leaves {
        let _ = writeln!(out, "    {},", leaf.kebab());
    }
    out.push_str("}\n");

    for leaf in leaves.iter().filter(|leaf| !leaf.fields.is_empty()) {
        let _ = writeln!(out, "\nexport struct {} {{", leaf.params());
        for field in &leaf.fields {
            let _ = writeln!(out, "    {}: {},", kebab(&field.name.to_string()), field.slint);
        }
        out.push_str("}\n");
    }

    out.push_str("\nexport global Navigator {\n    in property <RouteId> route;\n");
    for leaf in leaves.iter().filter(|leaf| !leaf.fields.is_empty()) {
        let _ = writeln!(out, "    in property <{}> {}-params;", leaf.params(), leaf.kebab());
    }
    out.push_str(
        "    in property <bool> can-go-back;\n    in property <bool> can-go-forward;\n    \
         in property <bool> asking;\n    in property <string> question;\n    \
         in property <string> confirm;\n    in property <string> cancel;\n    \
         in property <string> failure;\n",
    );
    for leaf in leaves {
        let takes = match leaf.fields.is_empty() {
            true => String::new(),
            false => leaf.params(),
        };
        let _ = writeln!(out, "    callback go-{}({takes});", leaf.kebab());
    }
    out.push_str("    callback back();\n    callback forward();\n    callback answer(bool);\n}\n");
    out
}

fn snake(kebab: &str) -> String {
    kebab.replace('-', "_")
}

fn rust(leaves: &[Leaf], adapter: &str) -> String {
    let root = leaves
        .first()
        .and_then(|leaf| leaf.path.first())
        .map(|(root, _)| root.to_string())
        .unwrap_or_else(|| "Route".to_owned());

    let mut out = String::new();
    let _ = write!(
        out,
        "impl<W> {adapter}::Shows<W> for {root}
where
    W: ::slint::ComponentHandle,
    for<'a> Navigator<'a>: ::slint::Global<'a, W>,
{{
    fn show(window: &W, nav: &{adapter}::Navigator<Self>) {{
        let global = window.global::<Navigator>();
        let ask = nav.pending();
        global.set_can_go_back(nav.can_go_back());
        global.set_can_go_forward(nav.can_go_forward());
        global.set_asking(ask.is_some());
        let ask = ask.unwrap_or_else(|| {adapter}::Ask::new(\"\", \"\", \"\"));
        global.set_question(ask.text.into());
        global.set_confirm(ask.confirm.into());
        global.set_cancel(ask.cancel.into());
        global.set_failure(nav.failure().unwrap_or_default().into());

        match nav.current() {{
"
    );

    for leaf in leaves {
        let _ = writeln!(out, "            Some({}) => {{", leaf.pattern());
        if !leaf.fields.is_empty() {
            let fields: Vec<String> = leaf
                .fields
                .iter()
                .map(|field| format!("{0}: {adapter}::ToSlint::to_slint(&{0})", field.name))
                .collect();
            let _ = writeln!(
                out,
                "                global.set_{}_params({} {{ {} }});",
                snake(&leaf.kebab()),
                leaf.params(),
                fields.join(", ")
            );
        }
        let _ = writeln!(out, "                global.set_route(RouteId::{});", leaf.id());
        out.push_str("            }\n");
    }

    let _ = write!(
        out,
        "            None => {{}}
        }}
    }}

    fn listen(window: &W, nav: &{adapter}::Navigator<Self>) {{
        let global = window.global::<Navigator>();
"
    );

    for leaf in leaves {
        let takes = match leaf.fields.is_empty() {
            true => "",
            false => "params",
        };
        let _ = write!(
            out,
            "        global.on_go_{}({{
            let nav = nav.clone();
            move |{takes}| {{
                let _moved = nav.to({});
            }}
        }});
",
            snake(&leaf.kebab()),
            leaf.built_from_params(adapter)
        );
    }

    out.push_str(
        "        global.on_back({
            let nav = nav.clone();
            move || {
                nav.back();
            }
        });
        global.on_forward({
            let nav = nav.clone();
            move || {
                nav.forward();
            }
        });
        global.on_answer({
            let nav = nav.clone();
            move |allowed| nav.answer(allowed)
        });
    }
}
",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTES: &str = r#"
        use guinea::slint::Route;

        #[derive(Clone, PartialEq, Debug, Route)]
        pub enum Route {
            Home,
            #[at("/hosts/:name")]
            Host { name: String, port: u16 },
            Processes(ProcessesRoute),
        }

        #[derive(Clone, PartialEq, Debug, Route)]
        pub enum ProcessesRoute {
            List,
            One { pid: u32, load: f64, live: bool },
        }

        pub struct NotARoute;
    "#;

    fn generated() -> Generated {
        emit(ROUTES, "::guinea::slint").expect("the routes are written out")
    }

    #[test]
    fn every_place_is_one_name_in_the_enum_nested_ones_by_their_path() {
        let slint = generated().slint;

        assert!(
            slint.contains("export enum RouteId {\n    home,\n    host,\n    processes-list,\n    processes-one,\n}"),
            "{slint}"
        );
    }

    #[test]
    fn a_place_with_parameters_has_a_struct_and_a_property_for_them() {
        let slint = generated().slint;

        assert!(
            slint.contains("export struct ProcessesOneParams {\n    pid: int,\n    load: float,\n    live: bool,\n}"),
            "{slint}"
        );
        assert!(slint.contains("    in property <HostParams> host-params;"), "{slint}");
        assert!(!slint.contains("HomeParams"), "a place without parameters has none: {slint}");
    }

    #[test]
    fn every_place_has_a_callback_that_goes_there() {
        let slint = generated().slint;

        assert!(slint.contains("    callback go-home();"), "{slint}");
        assert!(slint.contains("    callback go-processes-one(ProcessesOneParams);"), "{slint}");
        assert!(slint.contains("    callback back();"), "{slint}");
        assert!(slint.contains("    callback answer(bool);"), "{slint}");
    }

    #[test]
    fn the_rust_side_builds_and_matches_a_nested_place_through_its_enums() {
        let rust = generated().rust;

        assert!(
            rust.contains(
                "nav.to(Route::Processes(ProcessesRoute::One { pid: ::guinea::slint::FromSlint::from_slint(params.pid), load: ::guinea::slint::FromSlint::from_slint(params.load), live: ::guinea::slint::FromSlint::from_slint(params.live) }))"
            ),
            "{rust}"
        );
        assert!(
            rust.contains("Some(Route::Processes(ProcessesRoute::One { pid, load, live })) => {"),
            "{rust}"
        );
        assert!(rust.contains("global.set_route(RouteId::ProcessesOne);"), "{rust}");
    }

    #[test]
    fn a_parameter_the_global_cannot_carry_is_refused_by_name() {
        let error = emit(
            "#[derive(Route)] enum Route { Process { pid: Pid } }",
            "::guinea::slint",
        )
        .err()
        .map(|error| error.to_string());

        assert_eq!(
            error.as_deref(),
            Some(
                "`Route::Process.pid` is a `Pid`, which the Navigator global cannot carry: \
                 a parameter is a String, a bool, an integer or a float"
            )
        );
    }

    #[test]
    fn a_nested_route_that_is_not_in_the_file_is_refused_by_name() {
        let error = emit("#[derive(Route)] enum Route { Settings(SettingsRoute) }", "::guinea::slint")
            .err()
            .map(|error| error.to_string());

        assert_eq!(
            error.as_deref(),
            Some("`Route::Settings` nests `SettingsRoute`, which is not a `#[derive(Route)]` enum in this file")
        );
    }

    #[test]
    fn two_places_with_one_name_are_refused() {
        let error = emit(
            "#[derive(Route)] enum Route { AB, A(Inner) } #[derive(Route)] enum Inner { B }",
            "::guinea::slint",
        )
        .err()
        .map(|error| error.to_string());

        assert_eq!(
            error.as_deref(),
            Some("`Route::AB` and `Route::A::B` are both `RouteId.a-b` in Slint: rename one")
        );
    }

    #[test]
    fn a_file_without_the_route_enum_says_so() {
        let error = emit("pub struct Nothing;", "::guinea::slint")
            .err()
            .map(|error| error.to_string());

        assert_eq!(error.as_deref(), Some("no `#[derive(Route)] enum Route` in it"));
    }
}
