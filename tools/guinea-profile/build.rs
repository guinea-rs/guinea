use std::{
    env, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

fn collect(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            files.push(path.strip_prefix(root).unwrap().to_owned());
        }
    }
}
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets");
    println!("cargo:rerun-if-changed=assets");
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    files.sort();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut generated = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for file in files {
        let name = file.to_str().unwrap().replace('\\', "/");
        name.hash(&mut hash);
        fs::read(root.join(&file)).unwrap().hash(&mut hash);
        generated.push_str(&format!(
            "({name:?}, include_bytes!({:?})),\n",
            root.join(file)
        ));
    }
    generated.push_str(&format!(
        "];\npub const FINGERPRINT: &str = \"{:016x}\";\n",
        hash.finish()
    ));
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("integration.rs"),
        generated,
    )
    .unwrap();
}
