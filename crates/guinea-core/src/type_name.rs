/// The name the trace and the snapshot give `T`: its type and the module it
/// sits in, without the rest of the path - and with its generic arguments,
/// written out in full, so `GenericAgentActor` for one agent is not taken for
/// the same actor for another: `agent::GenericAgentActor<uniproc_agent::windows::WindowsAgent>`.
pub fn short_type_name<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    let head = full.split('<').next().unwrap_or(full);

    let mut parts = head.rsplitn(3, "::");
    let start = match (parts.next(), parts.next(), parts.next()) {
        (Some(name), Some(module), Some(_)) => head.len() - module.len() - name.len() - "::".len(),
        _ => 0,
    };

    &full[start..]
}

#[cfg(test)]
mod tests {
    use super::short_type_name;

    mod agent {
        pub struct Agent<T>(pub T);
        pub struct Plain;
    }

    mod windows {
        pub struct WindowsAgent;
    }

    #[test]
    fn a_name_keeps_its_module_and_drops_the_rest_of_the_path() {
        assert_eq!(short_type_name::<agent::Plain>(), "agent::Plain");
    }

    #[test]
    fn a_generic_name_keeps_its_arguments_in_full() {
        assert_eq!(
            short_type_name::<agent::Agent<windows::WindowsAgent>>(),
            "agent::Agent<guinea_core::type_name::tests::windows::WindowsAgent>"
        );
        assert_ne!(
            short_type_name::<agent::Agent<windows::WindowsAgent>>(),
            short_type_name::<agent::Agent<agent::Plain>>(),
            "one generic actor per argument is told apart"
        );
    }

    #[test]
    fn a_name_with_no_module_is_left_as_it_is() {
        assert_eq!(short_type_name::<u32>(), "u32");
        assert_eq!(short_type_name::<Vec<u32>>(), "vec::Vec<u32>", "the path, not the arguments");
    }
}
