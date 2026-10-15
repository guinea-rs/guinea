//! What the tool watching an application is told of its windows when it
//! starts: what was mounted where, and where the navigators are.

use guinea_core::observability::observer::{Describer, Event};
use guinea_core::scope::Scope;

fn describe(scope: Scope, events: &mut Vec<Event>) {
    crate::mount::describe(scope, events);
    crate::navigator::describe(scope, events);
}

inventory::submit! { Describer(describe) }

/// The name a page or a layout goes by: its type, without the path or the
/// generics.
pub fn short(name: &'static str) -> &'static str {
    let generic = name.find('<').unwrap_or(name.len());
    let start = name[..generic].rfind("::").map_or(0, |at| at + 2);
    &name[start..generic]
}

#[cfg(test)]
mod tests {
    use super::short;

    #[test]
    fn names_lose_their_path_and_keep_their_generics_out() {
        assert_eq!(short("app::pages::Processes"), "Processes");
        assert_eq!(short("guinea_eframe::Egui"), "Egui");
        assert_eq!(short("a::List<b::Recent>"), "List");
        assert_eq!(short("Plain"), "Plain");
    }
}
