use std::{fs, io::Write, process::Command};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_guinea-profile"))
}

#[test]
fn relocated_cli_dry_run_needs_no_cargo_or_checkout_in_working_directory() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp
        .path()
        .join(format!("profiler{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_guinea-profile"), &binary).unwrap();
    let output = Command::new(&binary)
        .current_dir(temp.path())
        .env("PATH", "")
        .args(["record", "--dry-run", "--no-ui", "--"])
        .arg(&binary)
        .args(["--filter", "a b; $(test)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.path().join("profiles").exists());
}

#[test]
fn standalone_process_preserves_inputs_and_rejects_overwriting_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let sink = guinea_trace::Sink::new();
    let recording = guinea_trace::perfetto::Recording::start(&sink, Vec::new()).unwrap();
    let guinea = recording.finish().unwrap();
    fs::write(temp.path().join("guinea.pftrace"), &guinea).unwrap();
    let start = guinea_trace::started_at()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0;
    let profile = serde_json::json!({"meta":{"startTime":start,"interval":10.0},"threads":[]});
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&serde_json::to_vec(&profile).unwrap())
        .unwrap();
    let cpu = encoder.finish().unwrap();
    fs::write(temp.path().join("cpu.json.gz"), &cpu).unwrap();
    let output = cli()
        .current_dir(temp.path())
        .env("PATH", "")
        .arg("process")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for file in ["combined.pftrace", "raw.pftrace", "cpu.perfetto.json.gz"] {
        assert!(temp.path().join(file).is_file(), "{file}");
    }
    let combined = fs::read(temp.path().join("combined.pftrace")).unwrap();
    let output = cli().arg("process").arg(temp.path()).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read(temp.path().join("combined.pftrace")).unwrap(),
        combined
    );
    assert_eq!(fs::read(temp.path().join("cpu.json.gz")).unwrap(), cpu);
    assert_eq!(
        fs::read(temp.path().join("guinea.pftrace")).unwrap(),
        guinea
    );
}

#[test]
fn invalid_cli_arguments_fail_before_recording() {
    for args in [
        vec!["record"],
        vec!["record", "--rate", "0", "app"],
        vec!["record", "--duration", "0", "app"],
        vec!["view"],
        vec!["process"],
        vec![
            "--ui-dir",
            "ui",
            "--integration",
            "source",
            "view",
            "session",
        ],
    ] {
        assert!(!cli().args(args).output().unwrap().status.success());
    }
}

#[test]
fn viewer_serves_prebuilt_ui_without_build_tools() {
    use std::{
        io::{BufRead, BufReader, Read},
        net::TcpStream,
        process::{Child, Stdio},
    };
    struct Running(Child);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let ui = temp.path().join("ui");
    fs::create_dir(&ui).unwrap();
    fs::write(ui.join("index.html"), "prebuilt-perfetto").unwrap();
    fs::write(temp.path().join("combined.pftrace"), "combined").unwrap();
    fs::write(temp.path().join("raw.pftrace"), "raw").unwrap();
    let mut child = Running(
        cli()
            .env("PATH", "")
            .arg("--ui-dir")
            .arg(&ui)
            .args(["view", "--no-open"])
            .arg(temp.path())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut reader = BufReader::new(child.0.stderr.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let url = line
        .trim()
        .strip_prefix("Perfetto: http://")
        .expect("viewer URL");
    let address = url.split('/').next().unwrap();
    for (path, body) in [
        ("/", "prebuilt-perfetto"),
        ("/profile/combined.pftrace", "combined"),
        ("/profile/raw.pftrace", "raw"),
    ] {
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert_eq!(response.split_once("\r\n\r\n").unwrap().1, body);
    }
}
