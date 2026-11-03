//! Packaged copies retain the integration's repository-relative SQL paths.
use std::{
    fs,
    path::{Path, PathBuf},
};
include!(concat!(env!("OUT_DIR"), "/integration.rs"));

pub fn extract(cache: &Path) -> super::Result<PathBuf> {
    let root = cache.join(format!(
        "integration-{}-{FINGERPRINT}",
        env!("CARGO_PKG_VERSION")
    ));
    for (name, bytes) in FILES {
        let path = root.join(name);
        if fs::read(&path).ok().as_deref() != Some(*bytes) {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, bytes)?;
        }
    }
    Ok(root.join("tools/perfetto"))
}

/// Stage packaged sources inside WSL, independent of Windows drive mounts.
#[cfg(windows)]
pub fn wsl_build() -> super::Result<std::process::Output> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut archive = tar::Builder::new(Vec::new());
    for (name, bytes) in FILES {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, name, *bytes)?;
    }
    let archive = archive.into_inner()?;
    let identity = format!("integration-{}-{FINGERPRINT}", env!("CARGO_PKG_VERSION"));
    let mut child = Command::new("wsl")
        .args([
            "--exec",
            "bash",
            "-c",
            r#"set -euo pipefail
root="${XDG_CACHE_HOME:-$HOME/.cache}/guinea-profile/$1"
mkdir -p "$root"
exec 8>"$root.lock"
flock 8
tar -xf - -C "$root"
bash "$root/tools/perfetto/build.sh"
"#,
            "guinea-profile",
            &identity,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let written = child.stdin.take().unwrap().write_all(&archive);
    let output = child.wait_with_output()?;
    // Prefer the useful WSL diagnostic over a broken pipe from an early exit.
    if output.status.success() {
        written?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_integration_matches_sources() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        // Published crates have no surrounding repository; extraction is tested below.
        if !repository.join("tools/perfetto/build.sh").is_file() {
            return;
        }
        for (name, bytes) in FILES {
            assert_eq!(
                fs::read(repository.join(name)).unwrap(),
                *bytes,
                "stale bundled asset: {name}"
            );
        }
        for directory in ["src", "scripts", "sql"] {
            let source = repository.join("tools/perfetto").join(directory);
            for file in fs::read_dir(source).unwrap() {
                let name = format!(
                    "tools/perfetto/{directory}/{}",
                    file.unwrap().file_name().to_str().unwrap()
                );
                assert!(
                    FILES.iter().any(|(asset, _)| *asset == name),
                    "missing bundled asset: {name}"
                );
            }
        }
    }

    #[test]
    fn relocated_executable_extracts_without_checkout_or_tools() {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory
            .path()
            .join(format!("bundle-tests{}", std::env::consts::EXE_SUFFIX));
        fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
        let output = std::process::Command::new(binary)
            .current_dir(directory.path())
            .env("PATH", "")
            .args([
                "--exact",
                "integration::tests::extraction_is_self_contained_and_repairs_missing_assets",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[test]
    fn extraction_is_self_contained_and_repairs_missing_assets() {
        let cache = tempfile::tempdir().unwrap();
        let integration = extract(cache.path()).unwrap();
        assert!(integration.starts_with(cache.path()));
        for (name, bytes) in FILES {
            let root = integration.parent().unwrap().parent().unwrap();
            assert_eq!(fs::read(root.join(name)).unwrap(), *bytes);
        }
        let script = integration.join("build.sh");
        fs::remove_file(&script).unwrap();
        fs::write(integration.join("src/index.ts"), b"damaged").unwrap();
        assert_eq!(extract(cache.path()).unwrap(), integration);
        assert!(script.is_file());
        assert_eq!(
            fs::read(integration.join("src/index.ts")).unwrap(),
            FILES
                .iter()
                .find(|(name, _)| *name == "tools/perfetto/src/index.ts")
                .unwrap()
                .1
        );
    }
}
