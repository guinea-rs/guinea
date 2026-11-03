use clap::{Arg, ArgAction, ArgMatches, value_parser};
use std::{
    ffi::OsString,
    io::{self, BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn cli() -> clap::Command {
    clap::Command::new("cargo xtask profile")
        .about("Build the reference application with symbols and launch guinea-profile")
        .arg(Arg::new("package").short('p').long("package"))
        .arg(Arg::new("bin").long("bin").conflicts_with("example"))
        .arg(Arg::new("example").long("example"))
        .arg(Arg::new("features").long("features"))
        .arg(
            Arg::new("manifest-path")
                .long("manifest-path")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(Arg::new("no-ui").long("no-ui").action(ArgAction::SetTrue))
        .arg(
            Arg::new("no-open")
                .long("no-open")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("rate")
                .long("rate")
                .value_parser(value_parser!(u32).range(1..)),
        )
        .arg(
            Arg::new("duration")
                .long("duration")
                .value_parser(value_parser!(u32).range(1..)),
        )
        .arg(
            Arg::new("args")
                .last(true)
                .num_args(0..)
                .value_parser(value_parser!(OsString)),
        )
}
pub fn run() -> Result<()> {
    let options = cli().get_matches_from(std::env::args_os().skip(1));
    let executable = build(&options)?;
    let mut command = Command::new("cargo");
    command
        .args(["run", "--locked", "--manifest-path"])
        .arg(super::root().join("Cargo.toml"))
        .args(["-p", "guinea-profile", "--", "record"]);
    for key in ["no-ui", "no-open", "dry-run"] {
        if options.get_flag(key) {
            command.arg(format!("--{key}"));
        }
    }
    if let Some(output) = options.get_one::<PathBuf>("output") {
        command.arg("--output").arg(output);
    }
    for key in ["rate", "duration"] {
        if let Some(value) = options.get_one::<u32>(key) {
            command.arg(format!("--{key}")).arg(value.to_string());
        }
    }
    command.arg("--").arg(executable);
    if let Some(args) = options.get_many::<OsString>("args") {
        command.args(args);
    }
    if !command.status()?.success() {
        return Err("guinea-profile failed".into());
    }
    Ok(())
}
fn default_target(options: &ArgMatches) -> bool {
    !["package", "bin", "example", "manifest-path"]
        .iter()
        .any(|key| options.contains_id(key))
}

fn manifest_path(options: &ArgMatches) -> Option<PathBuf> {
    options
        .get_one::<PathBuf>("manifest-path")
        .cloned()
        .or_else(|| {
            default_target(options).then(|| super::root().join("examples/todo-core/Cargo.toml"))
        })
}

fn build(options: &ArgMatches) -> Result<PathBuf> {
    let mut command = Command::new("cargo");
    // Also works with standalone workspaces which do not define our profile.
    command.args([
        "--config",
        "profile.profiling.inherits=\"release\"",
        "--config",
        "profile.profiling.debug=2",
        "--config",
        "profile.profiling.strip=false",
        "--config",
        "profile.profiling.opt-level=3",
        "build",
        "--locked",
        "--profile",
        "profiling",
        "--message-format=json-render-diagnostics",
    ]);
    if default_target(options) {
        command.args(["-p", "todo-core", "--bin", "todo-profile", "--features"]);
        command.arg(match options.get_one::<String>("features") {
            Some(features) => format!("profile,{features}"),
            None => "profile".into(),
        });
    }
    for key in ["package", "bin", "example", "features"] {
        if key == "features" && default_target(options) {
            continue;
        }
        if let Some(value) = options.get_one::<String>(key) {
            command.arg(format!("--{key}")).arg(value);
        }
    }
    if let Some(path) = manifest_path(options) {
        command.arg("--manifest-path").arg(path);
    }
    #[cfg(all(windows, target_env = "msvc"))]
    {
        // Cargo profile `debug = 2` cannot override a linker /DEBUG:NONE from
        // target rustflags. Opt in for this child build only, preserving flags
        // the caller explicitly supplied, including encoded flags with spaces.
        let (key, flags) = windows_symbol_flags(
            std::env::var_os("CARGO_ENCODED_RUSTFLAGS"),
            std::env::var_os("RUSTFLAGS"),
        );
        let mut metadata = Command::new("cargo");
        metadata.args(["metadata", "--no-deps", "--format-version", "1", "--locked"]);
        if let Some(path) = manifest_path(options) {
            metadata.arg("--manifest-path").arg(path);
        }
        let output = metadata.output()?;
        if !output.status.success() {
            return Err(format!(
                "cannot locate profiling symbols: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let mut directory = PathBuf::from(
            metadata["target_directory"]
                .as_str()
                .ok_or("missing Cargo target_directory")?,
        )
        .join("profiling");
        if options.contains_id("example") {
            directory.push("examples");
        }
        command.env(
            "CARGO_ENCODED_RUSTFLAGS",
            with_pdb_directory(key, flags, &directory),
        );
    }
    command.stdout(Stdio::piped());
    let mut child = command.spawn()?;
    let mut executables = Vec::new();
    let result: io::Result<()> = (|| {
        for line in BufReader::new(child.stdout.take().unwrap()).lines() {
            let line = line?;
            let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if message["reason"] == "compiler-message" {
                if let Some(rendered) = message["message"]["rendered"].as_str() {
                    eprint!("{rendered}");
                }
            }
            if let Some(executable) = artifact(&message) {
                executables.push(executable);
            }
        }
        Ok(())
    })();
    let status = child.wait()?;
    result?;
    if !status.success() {
        return Err(format!("profiling build failed: {status}").into());
    }
    executables.sort();
    executables.dedup();
    if executables.len() != 1 {
        return Err("select exactly one executable with --package and --bin or --example".into());
    }
    Ok(executables.remove(0))
}

#[cfg(all(windows, target_env = "msvc"))]
fn windows_symbol_flags(
    encoded: Option<OsString>,
    plain: Option<OsString>,
) -> (&'static str, OsString) {
    if let Some(mut flags) = encoded {
        if !flags.is_empty() {
            flags.push("\u{1f}");
        }
        flags.push("-C\u{1f}link-arg=/DEBUG:FULL");
        ("CARGO_ENCODED_RUSTFLAGS", flags)
    } else {
        let mut flags = plain.unwrap_or_default();
        if !flags.is_empty() {
            flags.push(" ");
        }
        flags.push("-C link-arg=/DEBUG:FULL");
        ("RUSTFLAGS", flags)
    }
}

#[cfg(all(windows, target_env = "msvc"))]
fn with_pdb_directory(key: &str, flags: OsString, directory: &std::path::Path) -> OsString {
    let mut encoded = if key == "CARGO_ENCODED_RUSTFLAGS" {
        flags
    } else {
        flags
            .to_string_lossy()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("\u{1f}")
            .into()
    };
    // samply 0.13 presymbolication ignores --symbol-dir. An absolute RSDS path
    // lets its existing symbol manager find the PDB without changing app CWD.
    encoded.push("\u{1f}-C\u{1f}link-arg=/PDBALTPATH:");
    encoded.push(directory);
    encoded.push("/%_PDB%");
    encoded
}

fn artifact(message: &serde_json::Value) -> Option<PathBuf> {
    if message["reason"] != "compiler-artifact" {
        return None;
    }
    let kinds = message["target"]["kind"].as_array()?;
    if !kinds.iter().any(|kind| kind == "bin" || kind == "example") {
        return None;
    }
    message["executable"].as_str().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(windows, target_env = "msvc"))]
    #[test]
    fn pdb_path_with_spaces_is_one_encoded_linker_argument() {
        let encoded = with_pdb_directory(
            "RUSTFLAGS",
            "-C link-arg=/DEBUG:FULL".into(),
            std::path::Path::new("C:/a b/profiling/examples"),
        );
        let args: Vec<_> = encoded.to_str().unwrap().split('\u{1f}').collect();
        assert_eq!(
            args,
            [
                "-C",
                "link-arg=/DEBUG:FULL",
                "-C",
                "link-arg=/PDBALTPATH:C:/a b/profiling/examples/%_PDB%"
            ]
        );
    }

    #[cfg(all(windows, target_env = "msvc"))]
    #[test]
    fn profiling_enables_pdbs_and_preserves_explicit_rustflags() {
        assert_eq!(
            windows_symbol_flags(None, None),
            ("RUSTFLAGS", "-C link-arg=/DEBUG:FULL".into())
        );
        assert_eq!(
            windows_symbol_flags(None, Some("-C target-cpu=native".into())),
            (
                "RUSTFLAGS",
                "-C target-cpu=native -C link-arg=/DEBUG:FULL".into()
            )
        );
        assert_eq!(
            windows_symbol_flags(
                Some("--remap-path-prefix\u{1f}C:/a b=source".into()),
                Some("ignored".into())
            ),
            (
                "CARGO_ENCODED_RUSTFLAGS",
                "--remap-path-prefix\u{1f}C:/a b=source\u{1f}-C\u{1f}link-arg=/DEBUG:FULL".into()
            )
        );
    }

    #[test]
    fn the_default_target_is_the_shared_todo_application() {
        let options = cli().try_get_matches_from(["profile"]).unwrap();
        assert_eq!(
            manifest_path(&options),
            Some(super::super::root().join("examples/todo-core/Cargo.toml"))
        );
        let explicit = cli()
            .try_get_matches_from(["profile", "-p", "guinea-trace", "--example", "perfetto_cpu"])
            .unwrap();
        assert!(!default_target(&explicit));
        assert_eq!(manifest_path(&explicit), None);
    }

    #[test]
    fn build_scripts_and_libraries_are_not_profile_targets() {
        for kind in ["custom-build", "lib", "proc-macro"] {
            assert_eq!(
                artifact(&serde_json::json!({"reason":"compiler-artifact",
                "target":{"kind":[kind]},"executable":"wrong"})),
                None
            );
        }
        assert_eq!(
            artifact(&serde_json::json!({"reason":"compiler-artifact",
            "target":{"kind":["example"]},"executable":"correct"})),
            Some(PathBuf::from("correct"))
        );
    }
}
