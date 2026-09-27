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
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let manifest_path = Path::new(&manifest_dir).join(manifest_relative_path);

    println!("cargo:rerun-if-changed={}", manifest_path.display());

    let contents = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", manifest_path.display()));
    let manifest: Manifest = toml::from_str(&contents)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", manifest_path.display()));

    let icon_path = manifest_path
        .parent()
        .expect("manifest path has no parent directory")
        .join(&manifest.window.icon);
    println!("cargo:rerun-if-changed={}", icon_path.display());

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_windows_resources(&manifest, &icon_path);
    }

    write_generated_consts(&manifest, &icon_path);
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

fn write_generated_consts(manifest: &Manifest, icon_path: &Path) {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest = Path::new(&out_dir).join("guinea_meta.rs");

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

    std::fs::write(&dest, generated)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", dest.display()));
}
