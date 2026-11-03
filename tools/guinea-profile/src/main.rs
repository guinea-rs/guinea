use clap::{Arg, ArgAction, Command, value_parser};
use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
mod integration;
mod profile_ui;
mod record;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
struct Viewer {
    ui_dir: Option<PathBuf>,
    integration: Option<PathBuf>,
    cache: PathBuf,
}
fn record_cli() -> Command {
    Command::new("record")
        .about("Record an existing executable with samply and Guinea, then open Perfetto")
        .arg(
            Arg::new("output")
                .long("output")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("rate")
                .long("rate")
                .default_value("100")
                .value_parser(value_parser!(u32).range(1..)),
        )
        .arg(
            Arg::new("duration")
                .long("duration")
                .value_parser(value_parser!(u32).range(1..)),
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
            Arg::new("executable")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("args")
                .num_args(0..)
                .allow_hyphen_values(true)
                .value_parser(value_parser!(OsString)),
        )
}
fn cli() -> Command {
    let directory = || {
        Arg::new("directory")
            .required(true)
            .value_parser(value_parser!(PathBuf))
    };
    Command::new("guinea-profile")
        .subcommand_required(true)
        .arg(
            Arg::new("cache-dir")
                .long("cache-dir")
                .global(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("integration")
                .long("integration")
                .global(true)
                .value_parser(value_parser!(PathBuf))
                .help("Perfetto integration source directory (for building the UI)"),
        )
        .arg(
            Arg::new("ui-dir")
                .long("ui-dir")
                .global(true)
                .value_parser(value_parser!(PathBuf))
                .conflicts_with("integration")
                .help("Prebuilt Perfetto UI, no source checkout or UI build required"),
        )
        .subcommand(record_cli())
        .subcommand(
            Command::new("process")
                .visible_alias("merge")
                .about("Combine CPU and Guinea recordings")
                .arg(directory()),
        )
        .subcommand(
            Command::new("view")
                .about("Open an existing recording in Perfetto")
                .arg(directory())
                .arg(
                    Arg::new("no-open")
                        .long("no-open")
                        .action(ArgAction::SetTrue),
                ),
        )
}
fn main() -> Result<()> {
    let args = cli().get_matches();
    let cache = args
        .get_one::<PathBuf>("cache-dir")
        .cloned()
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
                .unwrap_or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(std::env::temp_dir)
                        .join(".cache")
                })
                .join("guinea-profile")
        });
    let viewer = Viewer {
        ui_dir: args.get_one::<PathBuf>("ui-dir").cloned(),
        integration: args.get_one::<PathBuf>("integration").cloned(),
        cache: cache.clone(),
    };
    match args.subcommand().unwrap() {
        ("record", options) => record::run(options, &cache, &viewer),
        ("process", options) => process(options.get_one::<PathBuf>("directory").unwrap()),
        ("view", options) => profile_ui::serve(
            options.get_one::<PathBuf>("directory").unwrap(),
            options.get_flag("no-open"),
            &viewer,
        ),
        _ => unreachable!(),
    }
}
fn process(directory: &Path) -> Result<()> {
    if directory.join("combined.pftrace").exists()
        || directory.join("raw.pftrace").exists()
        || directory.join("cpu.perfetto.json.gz").exists()
    {
        return Err("processed outputs already exist; source recordings preserved".into());
    }
    let combined = directory.join("combined.pftrace");
    let profile: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
        fs::File::open(directory.join("cpu.json.gz"))?,
    ))?;
    let symbols = match fs::File::open(directory.join("cpu.json.syms.json")) {
        Ok(file) => Some(serde_json::from_reader::<_, serde_json::Value>(file)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let guinea = fs::read(directory.join("guinea.pftrace"))?;
    let bytes = guinea_trace::perfetto::merge_cpu(&profile, symbols.as_ref(), &guinea)?;
    let raw = guinea_trace::perfetto::merge_cpu_raw(&profile, symbols.as_ref(), &guinea)?;
    record::prepare_perfetto(directory)?;
    fs::File::create_new(&combined)?.write_all(&bytes)?;
    fs::File::create_new(directory.join("raw.pftrace"))?.write_all(&raw)?;
    eprintln!("Profile: {}", combined.display());
    Ok(())
}
