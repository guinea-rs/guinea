/// Who this application is.
///
/// One source for the handful of strings that several plugins need and that
/// nothing else can supply: the store wants a directory to write in, an
/// updater wants a version to compare, a crash reporter wants a publisher.
/// Passing them plugin by plugin means the same string typed twice, and when
/// the copies drift the data quietly lands in two places.
///
/// The values come from the application's manifest through `guinea-meta`,
/// which generates them as constants at build time - see [`crate::app_meta`].
/// Nothing here is guessed from the Cargo package: an application's identity
/// outlives the crate that happens to build it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMeta {
    /// Shown to a person: "Processes".
    pub name: &'static str,
    /// Written for a machine, and stable across renames: reverse-DNS, as in
    /// "dev.uniproc.processes". This is what decides where data lives.
    pub identifier: &'static str,
    pub version: &'static str,
    pub publisher: &'static str,
    /// What the window is called. Manifest data rather than a UI concern -
    /// the backend that opens a window decides whether to use it.
    pub window_title: &'static str,
    /// The application icon, as the bytes of an `.ico`. Empty when the
    /// manifest had none.
    pub window_icon: &'static [u8],
}

impl AppMeta {
    /// The four strings that identify an application. Window title defaults to
    /// the name, and there is no icon - both are what a manifest adds.
    pub const fn new(
        name: &'static str,
        identifier: &'static str,
        version: &'static str,
        publisher: &'static str,
    ) -> Self {
        Self {
            name,
            identifier,
            version,
            publisher,
            window_title: name,
            window_icon: &[],
        }
    }
}

/// This application's identity, read from its manifest.
///
/// ```ignore
/// fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window> {
///     let meta = app_meta!();
///     app.plugin::<SingleInstancePlugin>(meta.clone())?;
///     Ok(Window::of::<Shell>().title(meta.window_title))
/// }
/// ```
///
/// An expression, so it goes straight into the call that needs it. The
/// constants `guinea-meta` generated at build time are included inside the
/// macro's own block - it expands where it is called, so `OUT_DIR` is the
/// application's, and nothing has to be declared at the crate root first.
#[macro_export]
macro_rules! app_meta {
    () => {{
        mod generated {
            include!(concat!(env!("OUT_DIR"), "/guinea_meta.rs"));
        }

        $crate::app::AppMeta {
            name: generated::APP_NAME,
            identifier: generated::APP_IDENTIFIER,
            version: generated::APP_VERSION,
            publisher: generated::APP_PUBLISHER,
            window_title: generated::WINDOW_TITLE,
            window_icon: generated::WINDOW_ICON,
        }
    }};
}
