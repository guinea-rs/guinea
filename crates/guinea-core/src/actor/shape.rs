//! What an actor declared in `actor!`, kept for devtools.
//!
//! Every edge is already known at compile time - `actor!` checks them - so
//! this is the same list, written down as data.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Send,
    Bg,
    Emit,
    Ask,
}

/// A type's name as devtools show it: its module and itself.
pub type Name = fn() -> &'static str;

pub fn name<T: ?Sized>() -> &'static str {
    crate::short_type_name::<T>()
}

#[derive(Clone, Copy)]
pub struct Edge {
    pub channel: Channel,
    pub target: Name,
    pub looping: bool,
}

#[derive(Clone, Copy)]
pub struct Handles {
    pub message: Name,
    /// `None` when the handler declared nothing about what it sends.
    pub edges: Option<&'static [Edge]>,
    /// Where the handler was written, when `#[handler]` wrote it.
    pub declared: Option<Declared>,
}

/// Where an `actor!` was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declared {
    /// As `file!()` gives it: relative to wherever the compiler ran, which is
    /// usually the workspace root.
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
    /// The declaring crate's manifest directory.
    pub crate_dir: &'static str,
}

/// Where the line that calls it was written.
///
/// `file!` and friends expand at the call site, so a macro that puts this in
/// what it generates points at the code the author wrote, not at itself.
#[macro_export]
macro_rules! declared {
    () => {
        $crate::actor::shape::Declared {
            file: ::core::file!(),
            line: ::core::line!(),
            column: ::core::column!(),
            crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
        }
    };
}

impl Declared {
    /// The file on this machine: `file` itself when absolute, otherwise the
    /// first of `crate_dir` and its ancestors that has it. `None` when the
    /// sources are not here.
    pub fn path(&self) -> Option<std::path::PathBuf> {
        let file = std::path::Path::new(self.file);
        if file.is_absolute() {
            return file.exists().then(|| file.to_path_buf());
        }
        std::path::Path::new(self.crate_dir)
            .ancestors()
            .map(|dir| dir.join(file))
            .find(|path| path.exists())
    }
}

#[derive(Clone, Copy)]
pub struct Shape {
    pub handles: &'static [Handles],
    pub publishes: &'static [Name],
    pub subscribes: &'static [Name],
    pub declared: Option<Declared>,
}

impl Shape {
    pub const UNKNOWN: Shape = Shape {
        handles: &[],
        publishes: &[],
        subscribes: &[],
        declared: None,
    };
}

impl std::fmt::Debug for Shape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shape")
            .field(
                "handles",
                &self.handles.iter().map(|h| (h.message)()).collect::<Vec<_>>(),
            )
            .field(
                "publishes",
                &self.publishes.iter().map(|n| n()).collect::<Vec<_>>(),
            )
            .field(
                "subscribes",
                &self.subscribes.iter().map(|n| n()).collect::<Vec<_>>(),
            )
            .field("declared", &self.declared)
            .finish()
    }
}
