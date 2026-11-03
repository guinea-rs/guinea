#![cfg(feature = "perfetto")]

use std::process::Command;

#[test]
fn launcher_recording_environment() {
    if std::env::var_os("GUINEA_TEST_RECORDING_CHILD").is_some() {
        let sink = guinea_trace::Sink::new();
        let recording = guinea_trace::perfetto::from_env(&sink).unwrap();
        if let Some(recording) = recording {
            let _sink = guinea_trace::record_into(&sink);
            guinea_trace::mark(|| guinea_trace::Point::Action { message: "Profile" });
            recording.finish().unwrap();
            // A second recording must not truncate the first one.
            assert!(guinea_trace::perfetto::from_env(&sink).is_err());
        } else {
            assert!(std::env::var_os("GUINEA_TRACE_FILE").is_none());
        }
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guinea.pftrace");
    for enabled in [false, true] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "launcher_recording_environment"])
            .env("GUINEA_TEST_RECORDING_CHILD", "1")
            .env_remove("GUINEA_TRACE_FILE");
        if enabled {
            child.env("GUINEA_TRACE_FILE", &path);
        }
        assert!(child.status().unwrap().success());
        assert_eq!(path.is_file(), enabled);
    }
    use protobuf::Message;
    let trace =
        perfetto_protos::trace::Trace::parse_from_bytes(&std::fs::read(path).unwrap()).unwrap();
    assert!(trace.packet.iter().any(|packet| packet.has_track_event()));
}
