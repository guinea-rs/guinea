use super::Result;
use clap::{Arg, Command, value_parser};

const NAMES: [&str; 6] = [
    "cpu-hotspots",
    "cpu-modules",
    "cpu-threads",
    "cpu-compare",
    "cpu-uiautomation",
    "allocations",
];
pub fn command() -> Command {
    Command::new("sql")
        .about("Print executable Perfetto SQL to stdout; no trace or server required")
        .arg(Arg::new("report").required(true).value_parser(NAMES))
        .arg(Arg::new("pid").long("pid").value_parser(value_parser!(u32)))
        .arg(Arg::new("tid").long("tid").value_parser(value_parser!(u32)))
        .arg(Arg::new("from").long("from").default_value("0s"))
        .arg(Arg::new("to").long("to"))
        .arg(
            Arg::new("split")
                .long("split")
                .help("Comparison boundary; requires cpu-compare and --to"),
        )
        .arg(
            Arg::new("automation")
                .long("automation")
                .value_parser(["all", "include", "exclude"]),
        )
        .arg(
            Arg::new("group")
                .long("group")
                .value_parser(["operation", "action"])
                .help("allocations only; action includes causal descendants"),
        )
        .arg(
            Arg::new("limit")
                .long("limit")
                .default_value("50")
                .value_parser(value_parser!(u32).range(1..=10000)),
        )
}
pub fn catalog() -> &'static str {
    include_str!("../sql/catalog.txt")
}
pub fn hint() {
    eprintln!(
        "SQL reports: guinea-profile reports\nGenerate SQL: guinea-profile sql cpu-hotspots --from 5s --to 20s\nRun the emitted SQL with Perfetto MCP execute_sql or Perfetto's SQL query panel."
    );
}
fn time(value: &str) -> Result<u64> {
    let ns = u64::try_from(humantime::parse_duration(value)?.as_nanos())
        .map_err(|_| "time exceeds supported nanosecond range")?;
    if ns > i64::MAX as u64 {
        return Err("time exceeds SQL integer range".into());
    }
    Ok(ns)
}
pub fn render(args: &clap::ArgMatches) -> Result<String> {
    let report = args.get_one::<String>("report").unwrap();
    let start = time(args.get_one::<String>("from").unwrap())?;
    let end = args.get_one::<String>("to").map(|v| time(v)).transpose()?;
    if end.is_some_and(|v| v <= start) {
        return Err("--to must be greater than --from".into());
    }
    let split = args
        .get_one::<String>("split")
        .map(|v| time(v))
        .transpose()?;
    if report == "cpu-compare" {
        if !matches!((split,end),(Some(s),Some(e)) if start < s && s < e) {
            return Err("cpu-compare requires --from < --split < --to".into());
        }
    } else if split.is_some() {
        return Err("--split is only supported by cpu-compare".into());
    }
    if report != "allocations" && args.contains_id("group") {
        return Err("--group is only supported by allocations".into());
    }
    if report == "allocations" && args.contains_id("automation") {
        return Err("--automation is only supported by CPU reports".into());
    }
    let automation = args
        .get_one::<String>("automation")
        .map(String::as_str)
        .unwrap_or(if report == "cpu-uiautomation" {
            "include"
        } else {
            "all"
        });
    let grouping = args
        .get_one::<String>("group")
        .map(String::as_str)
        .unwrap_or("operation");
    let mut env = minijinja::Environment::new();
    env.add_template(
        "report",
        if report == "allocations" {
            include_str!("../sql/allocations.sql.jinja")
        } else {
            include_str!("../sql/cpu.sql.jinja")
        },
    )?;
    Ok(env.get_template("report")?.render(minijinja::context! {
        report, start, end, split, automation, grouping,
        pid => args.get_one::<u32>("pid"), tid => args.get_one::<u32>("tid"),
        limit => args.get_one::<u32>("limit").unwrap(),
    })?)
}
