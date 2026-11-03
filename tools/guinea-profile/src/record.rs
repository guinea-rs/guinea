use super::Result;
use clap::ArgMatches;
use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
pub fn run(options: &ArgMatches, cache: &std::path::Path, viewer: &super::Viewer) -> Result<()> {
    let current = std::env::current_dir()?;
    let output = options
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or_else(|| {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            current
                .join("profiles")
                .join(format!("{timestamp}-{}", std::process::id()))
        });
    let output = std::path::absolute(output)?;
    if output.exists() {
        return Err(format!("recording directory already exists: {}", output.display()).into());
    }
    let executable = fs::canonicalize(options.get_one::<PathBuf>("executable").unwrap())?;
    let samply = if options.get_flag("dry-run") {
        PathBuf::from("samply")
    } else {
        prepare_samply(cache)?
    };
    let mut command = {
        let configured = record_command(options, &output, &executable, cache);
        // record_command preserves native arguments; select the cached executable.
        let mut actual = Command::new(&samply);
        actual.args(configured.get_args());
        for (key, value) in configured.get_envs() {
            if let Some(value) = value {
                actual.env(key, value);
            }
        }
        actual
    };
    if options.get_flag("dry-run") {
        eprintln!("{command:?}");
        return Ok(());
    }
    eprintln!("Profiling: {}", executable.display());
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&output)?;
    fs::write(
        output.join("session.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1, "executable": executable,
            "arguments": options.get_many::<OsString>("args").into_iter().flatten()
                .map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(),
            "sample_rate_hz": options.get_one::<u32>("rate"),
            "cpu": "cpu.perfetto.json.gz", "samply": "cpu.json.gz",
            "guinea": "guinea.pftrace", "correlated": false
        }))?,
    )?;
    let status = command.status()?;
    eprintln!("Recording: {}", output.display());
    if !status.success() {
        return Err(
            format!("samply exited with {status}; partial files have been preserved").into(),
        );
    }
    if !output.join("cpu.json.gz").is_file() {
        return Err("samply did not produce cpu.json.gz".into());
    }

    if !output.join("guinea.pftrace").is_file() {
        prepare_perfetto(&output)?;
        eprintln!("No Guinea trace: attach perfetto::from_env(&sink) in the application.");
    } else {
        super::process(&output)?;
        let session_path = output.join("session.json");
        let mut session: serde_json::Value =
            serde_json::from_reader(fs::File::open(&session_path)?)?;
        session["correlated"] = true.into();
        session["combined"] = "combined.pftrace".into();
        session["ui"] = "rs.guinea.Profile".into();
        fs::write(session_path, serde_json::to_vec_pretty(&session)?)?;
        if !options.get_flag("no-ui") {
            super::profile_ui::serve(&output, options.get_flag("no-open"), viewer)?;
        }
    }
    Ok(())
}

fn prepare_samply(cache: &std::path::Path) -> Result<PathBuf> {
    if Command::new("samply")
        .arg("--version")
        .output()
        .is_ok_and(|out| {
            out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "samply 0.13.1"
        })
    {
        return Ok(PathBuf::from("samply"));
    }
    let cache = cache.join("samply-0.13.1");
    let executable = cache
        .join("bin")
        .join(format!("samply{}", std::env::consts::EXE_SUFFIX));
    if !executable.is_file() {
        let status = Command::new("cargo")
            .args(["install", "--locked", "--version", "0.13.1", "--root"])
            .arg(&cache)
            .arg("samply")
            .status()?;
        if !status.success() {
            return Err("samply installation failed".into());
        }
    }
    Ok(executable)
}

pub(crate) fn prepare_perfetto(output: &std::path::Path) -> Result<()> {
    let input = flate2::read::GzDecoder::new(fs::File::open(output.join("cpu.json.gz"))?);
    // Preserve the leading `meta` key: Perfetto sniffs the JSON prefix to select
    // its Gecko importer. Alphabetically reordering keys selects another parser.
    let mut profile: serde_json::Value = serde_json::from_reader(input)?;
    normalize_sample_times(&mut profile)?;
    let file = fs::File::create_new(output.join("cpu.perfetto.json.gz"))?;
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    serde_json::to_writer(&mut encoder, &profile)?;
    encoder.finish()?;
    Ok(())
}

// samply's processed format uses delta-encoded milliseconds; Perfetto's Gecko
// importer currently only reads the absolute `time` column. Keep the original
// profile and its symbol sidecar intact for samply and future symbolication.
fn normalize_sample_times(profile: &mut serde_json::Value) -> Result<()> {
    let threads = profile["threads"]
        .as_array_mut()
        .ok_or("CPU profile has no threads array")?;
    for thread in threads {
        let samples = &mut thread["samples"];
        if samples.get("time").is_some() {
            continue;
        }
        let Some(deltas) = samples["timeDeltas"].as_array() else {
            continue;
        };
        let mut timestamp = 0.0;
        let times = deltas
            .iter()
            .map(|delta| {
                timestamp += delta.as_f64().ok_or("invalid CPU sample time delta")?;
                serde_json::Number::from_f64(timestamp)
                    .map(serde_json::Value::Number)
                    .ok_or("non-finite CPU sample timestamp")
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if samples["stack"].as_array().map(Vec::len) != Some(times.len()) {
            return Err("CPU sample stack/time lengths differ".into());
        }
        samples["time"] = serde_json::Value::Array(times);
    }
    Ok(())
}

fn record_command(
    options: &ArgMatches,
    output: &std::path::Path,
    executable: &std::path::Path,
    cache: &std::path::Path,
) -> Command {
    let mut command = Command::new("samply");
    #[cfg(windows)]
    if std::env::var_os("_NT_SYMBOL_PATH").is_none() {
        // samply 0.13 presymbolication uses this standard path, rather than
        // the CLI symbol-server settings used by its interactive server.
        let mut symbols = OsString::from("srv*");
        symbols.push(cache.join("symbols"));
        symbols.push("*https://msdl.microsoft.com/download/symbols");
        command.env("_NT_SYMBOL_PATH", symbols);
    }
    command
        .args([
            "record",
            "--save-only",
            "--unstable-presymbolicate",
            "--rate",
        ])
        .arg(options.get_one::<u32>("rate").unwrap().to_string())
        .arg("--output")
        .arg(output.join("cpu.json.gz"));
    if let Some(duration) = options.get_one::<u32>("duration") {
        command.arg("--duration").arg(duration.to_string());
    }
    if let Some(directory) = executable
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        command.arg("--symbol-dir").arg(directory);
    }
    command.arg("--").arg(executable);
    if let Some(args) = options.get_many::<OsString>("args") {
        command.args(args);
    }
    command.env("GUINEA_TRACE_FILE", output.join("guinea.pftrace"));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_times_become_perfetto_sample_times_without_changing_stacks() {
        let mut profile = serde_json::json!({"threads":[
            {"samples":{"timeDeltas":[2.5, 1.0, 0.25],"stack":[1,null,2]}},
            {"samples":{"time":[7.0],"stack":[3]}}
        ]});
        normalize_sample_times(&mut profile).unwrap();
        assert_eq!(
            profile["threads"][0]["samples"]["time"],
            serde_json::json!([2.5, 3.5, 3.75])
        );
        assert_eq!(
            profile["threads"][0]["samples"]["stack"],
            serde_json::json!([1, null, 2])
        );
        assert_eq!(
            profile["threads"][1]["samples"]["time"],
            serde_json::json!([7.0])
        );
        let mut invalid =
            serde_json::json!({"threads":[{"samples":{"timeDeltas":[1],"stack":[]}}]});
        assert!(normalize_sample_times(&mut invalid).is_err());
    }

    #[test]
    fn profile_serialization_preserves_the_gecko_detection_prefix() {
        let profile: serde_json::Value =
            serde_json::from_str(r#"{"meta":{},"libs":[],"threads":[]}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&profile).unwrap(),
            r#"{"meta":{},"libs":[],"threads":[]}"#
        );
    }

    #[test]
    fn arguments_are_forwarded_without_shell_interpretation() {
        let options = super::super::record_cli()
            .try_get_matches_from([
                "profile",
                "--rate",
                "250",
                "--duration",
                "10",
                "--",
                "app.exe",
                "--filter",
                "a b; $(test)",
            ])
            .unwrap();
        let output = PathBuf::from("recording with spaces");
        let command = record_command(
            &options,
            &output,
            &PathBuf::from("app.exe"),
            &PathBuf::from("cache"),
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "record",
                "--save-only",
                "--unstable-presymbolicate",
                "--rate",
                "250",
                "--output",
                output.join("cpu.json.gz").to_str().unwrap(),
                "--duration",
                "10",
                "--",
                "app.exe",
                "--filter",
                "a b; $(test)"
            ]
        );
        assert_eq!(
            command
                .get_envs()
                .filter(|(key, _)| *key == "GUINEA_TRACE_FILE")
                .collect::<Vec<_>>(),
            vec![(
                std::ffi::OsStr::new("GUINEA_TRACE_FILE"),
                Some(output.join("guinea.pftrace").as_os_str())
            )]
        );
        #[cfg(windows)]
        {
            let configured = command
                .get_envs()
                .find(|(key, _)| *key == "_NT_SYMBOL_PATH");
            if std::env::var_os("_NT_SYMBOL_PATH").is_some() {
                assert_eq!(
                    configured, None,
                    "explicit symbol settings must be inherited"
                );
            } else {
                let expected = format!(
                    "srv*{}*https://msdl.microsoft.com/download/symbols",
                    PathBuf::from("cache").join("symbols").display()
                );
                assert_eq!(configured.unwrap().1, Some(std::ffi::OsStr::new(&expected)));
            }
        }
    }
}
