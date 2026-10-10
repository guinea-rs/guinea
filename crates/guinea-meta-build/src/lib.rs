use guinea_codegen::Build;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct Manifest {
    app: AppSection,
    window: WindowSection,
}

#[derive(Deserialize)]
struct AppSection {
    name: String,
    identifier: String,
    version: String,
    publisher: String,
}

#[derive(Deserialize)]
struct WindowSection {
    title: String,
    icon: String,
}

pub fn generate(manifest_relative_path: &str) {
    let mut build = Build::from_env().unwrap_or_else(|e| panic!("{e}"));

    let contents = build
        .read(manifest_relative_path)
        .unwrap_or_else(|e| panic!("{e}"));
    let manifest_path = build.manifest_dir().join(manifest_relative_path);
    let manifest: Manifest = toml::from_str(&contents)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", manifest_path.display()));

    let icon_path = build.track(
        manifest_path
            .parent()
            .expect("manifest path has no parent directory")
            .join(&manifest.window.icon),
    );

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_windows_resources(&manifest, &icon_path);
    }

    write_generated_consts(&build, &manifest, &icon_path);
}

#[cfg(windows)]
fn embed_windows_resources(manifest: &Manifest, icon_path: &Path) {
    let icon_path_str = icon_path
        .to_str()
        .unwrap_or_else(|| panic!("non-UTF8 icon path: {}", icon_path.display()));

    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon_path_str);
    res.set("ProductName", &manifest.app.name);
    res.set("FileDescription", &manifest.app.name);
    res.set("CompanyName", &manifest.app.publisher);
    res.set("ProductVersion", &manifest.app.version);
    res.set("FileVersion", &manifest.app.version);
    res.compile().expect("failed to embed windows exe resources");
}

#[cfg(not(windows))]
fn embed_windows_resources(_manifest: &Manifest, _icon_path: &Path) {}

fn write_generated_consts(build: &Build, manifest: &Manifest, icon_path: &Path) {
    let icon_path_str = icon_path
        .to_str()
        .unwrap_or_else(|| panic!("non-UTF8 icon path: {}", icon_path.display()));

    let generated = format!(
        r#"#[allow(dead_code)]
pub const APP_NAME: &str = {name:?};
#[allow(dead_code)]
pub const APP_IDENTIFIER: &str = {identifier:?};
#[allow(dead_code)]
pub const APP_VERSION: &str = {version:?};
#[allow(dead_code)]
pub const APP_PUBLISHER: &str = {publisher:?};
#[allow(dead_code)]
pub const WINDOW_TITLE: &str = {title:?};
#[allow(dead_code)]
pub const WINDOW_ICON: &[u8] = include_bytes!({icon_path:?});
"#,
        name = manifest.app.name,
        identifier = manifest.app.identifier,
        version = manifest.app.version,
        publisher = manifest.app.publisher,
        title = manifest.window.title,
        icon_path = icon_path_str,
    );

    build
        .write("guinea_meta.rs", generated)
        .unwrap_or_else(|e| panic!("{e}"));
}
