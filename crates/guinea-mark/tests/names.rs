use guinea_mark::Mark;
mod left {
    #[derive(guinea_macros::Mark)]
    pub enum Marks {
        Memory,
    }
}
mod right {
    #[derive(guinea_macros::Mark)]
    pub enum Marks {
        Memory,
    }
}
#[derive(guinea_macros::Mark)]
enum Other {
    Memory,
}
#[test]
fn identical_variants_and_enum_names_in_distinct_modules_do_not_collide() {
    assert_ne!(left::Marks::Memory.name(), right::Marks::Memory.name());
    assert_ne!(left::Marks::Memory.name(), Other::Memory.name());
    assert_eq!(
        left::Marks::Memory.name(),
        concat!(module_path!(), "::left::Marks::Memory")
    );
}
