//! The facade's manifest without the `winui` backend, for the publish job.

use std::fs;
use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table};

const WINDOWS: &str = r#"cfg(target_os = "windows")"#;

pub fn without_winui(root: &Path) -> Result<(), String> {
    let path = root.join("crates/guinea/Cargo.toml");
    let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut manifest: DocumentMut = text
        .parse()
        .map_err(|error| format!("{}: {error}", path.display()))?;

    for section in ["dependencies", "dev-dependencies"] {
        drop_windows_dependency(&mut manifest, section, "guinea-winui");
    }

    let features = manifest
        .get_mut("features")
        .and_then(Item::as_table_mut)
        .ok_or("the facade has no [features]")?;

    features.remove("winui");
    drop_from(features, "default", "winui");
    drop_from(features, "harness", "guinea-winui?/harness");

    let left = manifest.to_string();
    if left.contains("guinea-winui") || left.contains("\"winui\"") {
        return Err(format!(
            "{}: winui is still named after taking it out - the manifest grew a use this does not know",
            path.display()
        ));
    }

    fs::write(&path, left).map_err(|error| format!("{}: {error}", path.display()))
}

fn drop_windows_dependency(manifest: &mut DocumentMut, section: &str, name: &str) {
    let Some(target) = manifest.get_mut("target").and_then(Item::as_table_mut) else {
        return;
    };
    let Some(windows) = target.get_mut(WINDOWS).and_then(Item::as_table_mut) else {
        return;
    };
    let Some(dependencies) = windows.get_mut(section).and_then(Item::as_table_mut) else {
        return;
    };

    dependencies.remove(name);

    if dependencies.is_empty() {
        windows.remove(section);
    }
    if windows.is_empty() {
        target.remove(WINDOWS);
    }
}

fn drop_from(features: &mut Table, feature: &str, entry: &str) {
    let Some(list) = features.get_mut(feature).and_then(Item::as_array_mut) else {
        return;
    };

    let mut kept: Array = list
        .iter()
        .filter(|value| value.as_str() != Some(entry))
        .cloned()
        .collect();
    kept.fmt();
    *list = kept;
}
