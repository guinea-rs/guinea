use std::{fs, process::Command};
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_guinea-profile"))
}
#[test]
fn catalog_and_sql_work_without_environment_or_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let out = cli()
        .arg("reports")
        .env("PATH", "")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    insta::assert_snapshot!("catalog", String::from_utf8(out.stdout).unwrap());
    let out = cli()
        .args([
            "sql",
            "cpu-hotspots",
            "--pid",
            "1234",
            "--tid",
            "42",
            "--from",
            "5s",
            "--to",
            "20s",
            "--automation",
            "exclude",
        ])
        .env("PATH", "")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out);
    assert!(out.stderr.is_empty());
    insta::assert_snapshot!(
        "filtered_hotspots_sql",
        String::from_utf8(out.stdout).unwrap()
    );
}
#[test]
fn rejects_invalid_parameters_without_printing_executable_sql() {
    for args in [
        vec!["sql", "cpu-hotspots", "--pid", "1;DROP TABLE slice"],
        vec!["sql", "cpu-hotspots", "--from", "20s", "--to", "5s"],
        vec!["sql", "cpu-hotspots", "--from", "NaN"],
        vec!["sql", "cpu-compare", "--split", "3s"],
        vec!["sql", "cpu-threads", "--split", "3s"],
        vec!["sql", "allocations", "--automation", "exclude"],
        vec!["sql", "cpu-hotspots", "--group", "action"],
        vec!["sql", "cpu-hotspots", "--limit", "0"],
    ] {
        let out = cli().args(&args).output().unwrap();
        assert!(!out.status.success(), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}
#[test]
#[ignore = "requires official Trace Processor in TRACE_PROCESSOR"]
fn native_reports_deduplicate_recursion_and_preserve_missing_allocations() {
    use guinea_trace::perfetto::{Recording, merge_cpu_raw};
    let sink = guinea_trace::Sink::new();
    let recording = Recording::start(&sink, Vec::new()).unwrap();
    {
        let _sink = guinea_trace::record_into(&sink);
        let action = guinea_trace::enter(|| guinea_trace::Point::Action { message: "Fixture" });
        drop(action);
    }
    let start = guinea_trace::started_at()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0;
    let profile = serde_json::json!({"meta":{"startTime":start,"interval":1.0},"threads":[{
      "pid":1234,"tid":42,"samples":{"time":[1000.0,2000.0],"stack":[2,2],"threadCPUDelta":[1000,1000]},
      "stackTable":{"frame":[0,1,2],"prefix":[null,0,1]},"frameTable":{"func":[0,1,1],"address":[1,2,2]},
      "funcTable":{"name":[0,1],"resource":[-1,-1]},"stringArray":["app::main","app::work"]}]});
    let trace = merge_cpu_raw(&profile, None, &recording.finish().unwrap()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("fixture.pftrace");
    fs::write(&path, trace).unwrap();
    let mut results = String::new();
    for name in [
        "cpu-hotspots",
        "cpu-modules",
        "cpu-threads",
        "cpu-compare",
        "cpu-uiautomation",
        "allocations",
    ] {
        let mut command = cli();
        command.args(["sql", name, "--to", "3s"]);
        if name == "cpu-compare" {
            command.args(["--split", "1500ms"]);
        }
        let sql = command.output().unwrap();
        assert!(sql.status.success());
        let query = temp.path().join("query.sql");
        fs::write(&query, sql.stdout).unwrap();
        let out = Command::new(std::env::var_os("TRACE_PROCESSOR").expect("TRACE_PROCESSOR"))
            .args(["query", "--format", "csv", "--query-file"])
            .arg(&query)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        results.push_str(name);
        results.push('\n');
        results.push_str(&String::from_utf8(out.stdout).unwrap().replace('\r', ""));
    }
    insta::assert_snapshot!("native_reports", results);
}
