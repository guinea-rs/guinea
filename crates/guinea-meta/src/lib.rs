#[macro_export]
macro_rules! manifest {
    () => {
        include!(concat!(env!("OUT_DIR"), "/guinea_meta.rs"));
    };
}
