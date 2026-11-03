//! Serve the pinned stock Perfetto UI and a recording on the same origin.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tower_http::services::{ServeDir, ServeFile};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn distribution(viewer: &super::Viewer) -> Result<PathBuf> {
    if let Some(directory) = &viewer.ui_dir {
        if !directory.join("index.html").is_file() {
            return Err("--ui-dir must contain index.html".into());
        }
        return Ok(directory.clone());
    }
    let integration = match &viewer.integration {
        Some(directory) => directory.clone(),
        None => super::integration::extract(&viewer.cache)?,
    };
    let script = std::path::absolute(integration)?.join("build.sh");
    if !script.is_file() {
        return Err("Perfetto integration missing; pass --integration <tools/perfetto> or --ui-dir <built UI>".into());
    }
    #[cfg(windows)]
    let mut command = {
        let converted = Command::new("wsl")
            .args(["--exec", "wslpath", "-u"])
            .arg(&script)
            .output()?;
        if !converted.status.success() {
            return Err("Perfetto UI requires a working WSL Linux distribution".into());
        }
        let mut command = Command::new("wsl");
        command.args([
            "--exec",
            "bash",
            String::from_utf8(converted.stdout)?.trim(),
        ]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("bash");
        command.arg(script);
        command
    };
    let output = command.stderr(Stdio::inherit()).output()?;
    if !output.status.success() {
        return Err("Perfetto UI build failed; see the build output above".into());
    }
    let directory = PathBuf::from(String::from_utf8(output.stdout)?.trim());
    if !directory.join("index.html").is_file() {
        return Err(format!("Perfetto UI missing from {}", directory.display()).into());
    }
    Ok(directory)
}

pub fn serve(recording: &Path, no_open: bool, viewer: &super::Viewer) -> Result<()> {
    let recording = recording.canonicalize()?;
    if !recording.join("combined.pftrace").is_file() {
        return Err("recording is missing combined.pftrace".into());
    }
    let ui = distribution(viewer)?;
    tokio::runtime::Runtime::new()?.block_on(async {
        let app = axum::Router::new()
            .route_service("/profile/combined.pftrace", ServeFile::new(recording.join("combined.pftrace")))
            .route_service("/profile/raw.pftrace", ServeFile::new(recording.join("raw.pftrace")))
            .fallback_service(ServeDir::new(ui));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let url = format!("http://127.0.0.1:{port}/#!/?url=http%3A%2F%2F127.0.0.1%3A{port}%2Fprofile%2Fcombined.pftrace&enablePlugins=rs.guinea.Profile");
        eprintln!("Perfetto: {url}\nCtrl+C stops the local viewer server.");
        if !no_open { webbrowser::open(&url)?; }
        axum::serve(listener, app).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await?;
        Ok(())
    })
}
