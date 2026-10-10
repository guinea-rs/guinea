//! What a build script reads and writes, with cargo told as it happens.
//!
//! A build script that generates code has two chores besides generating it:
//! telling cargo which inputs to watch, and not touching an output that came
//! out the same - a rewritten file is a changed file, and everything that
//! `include!`s it is rebuilt. Both are easy to forget at one call site and
//! repeated at every other.
//!
//! [`Build`] does both where the work happens: an input is watched because it
//! was read through [`Build::read`], an output is left alone by
//! [`Build::write`] when its contents did not change.
//!
//! ```no_run
//! // build.rs
//! fn main() -> std::io::Result<()> {
//!     let mut build = guinea_codegen::Build::from_env();
//!     let manifest = build.read("app.toml")?;
//!     build.write("app.rs", format!("pub const APP: &str = {manifest:?};"))?;
//!     Ok(())
//! }
//! ```

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

/// The package a build script runs for: where its sources are, where its
/// outputs go, and which inputs cargo has been told to watch.
#[derive(Debug)]
pub struct Build {
    manifest_dir: PathBuf,
    out_dir: PathBuf,
    tracked: Vec<PathBuf>,
}

impl Build {
    /// The package cargo is running this build script for.
    ///
    /// # Panics
    ///
    /// Outside a build script, where cargo has not set `CARGO_MANIFEST_DIR`
    /// and `OUT_DIR`.
    pub fn from_env() -> Self {
        Self::new(from_cargo("CARGO_MANIFEST_DIR"), from_cargo("OUT_DIR"))
    }

    fn new(manifest_dir: PathBuf, out_dir: PathBuf) -> Self {
        Self {
            manifest_dir,
            out_dir,
            tracked: Vec::new(),
        }
    }

    /// The directory of the package's `Cargo.toml`, which relative inputs are
    /// read from.
    pub fn manifest_dir(&self) -> &Path {
        &self.manifest_dir
    }

    /// Where outputs go.
    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }

    /// Has cargo rerun the build script when `path` changes, and returns it
    /// resolved against [`Self::manifest_dir`]. A directory is watched with
    /// everything under it.
    pub fn track(&mut self, path: impl AsRef<Path>) -> PathBuf {
        let path = self.manifest_dir.join(path);
        println!("cargo::rerun-if-changed={}", path.display());

        self.tracked.push(path.clone());
        path
    }

    /// Reads `path`, resolved and watched as [`Self::track`] does.
    pub fn read(&mut self, path: impl AsRef<Path>) -> io::Result<String> {
        let path = self.track(path);
        fs::read_to_string(&path).map_err(|e| naming(&path, e))
    }

    /// Writes `name` into [`Self::out_dir`] unless it already holds
    /// `contents`, and returns where it is.
    pub fn write(&self, name: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<PathBuf> {
        let path = self.out_dir.join(name);
        write_if_changed(&path, contents)?;
        Ok(path)
    }

    /// Sets an environment variable for the compilation of this package,
    /// where `env!` reads it.
    pub fn rustc_env(&self, key: &str, value: &str) {
        println!("cargo::rustc-env={key}={value}");
    }

    /// Every input watched so far, in the order it was.
    pub fn tracked(&self) -> &[PathBuf] {
        &self.tracked
    }
}

/// Writes `contents` to `path`, making the directories on the way, unless
/// the file already holds exactly that. Returns whether it wrote.
pub fn write_if_changed(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<bool> {
    let (path, contents) = (path.as_ref(), contents.as_ref());
    if fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| naming(path, e))?;
    }
    fs::write(path, contents).map_err(|e| naming(path, e))?;
    Ok(true)
}

fn naming(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

fn from_cargo(key: &str) -> PathBuf {
    match env::var_os(key) {
        Some(value) => PathBuf::from(value),
        None => panic!("{key}: not run by cargo"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn long_ago() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)
    }

    fn age(path: &Path) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(long_ago())
            .unwrap();
    }

    fn modified(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

    fn outcome<T>(result: io::Result<T>) -> Result<T, String> {
        result.map_err(|e| e.to_string())
    }

    #[test]
    fn outside_a_build_script_it_says_what_cargo_did_not_set() {
        let outcome = std::panic::catch_unwind(Build::from_env);

        let message = outcome
            .err()
            .and_then(|panic| panic.downcast::<String>().ok())
            .map(|message| *message);
        assert_eq!(message.as_deref(), Some("OUT_DIR: not run by cargo"));
    }

    #[test]
    fn writing_what_is_already_there_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generated.rs");
        fs::write(&path, "pub const A: u8 = 1;").unwrap();
        age(&path);

        assert_eq!(outcome(write_if_changed(&path, "pub const A: u8 = 1;")), Ok(false));

        assert_eq!(modified(&path), long_ago());
    }

    #[test]
    fn writing_something_else_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generated.rs");
        fs::write(&path, "pub const A: u8 = 1;").unwrap();

        assert_eq!(outcome(write_if_changed(&path, "pub const A: u8 = 2;")), Ok(true));

        assert_eq!(fs::read_to_string(&path).unwrap(), "pub const A: u8 = 2;");
    }

    #[test]
    fn writing_makes_the_directories_on_the_way() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("generated.rs");

        assert_eq!(outcome(write_if_changed(&path, "x")), Ok(true));

        assert_eq!(fs::read_to_string(&path).unwrap(), "x");
    }

    #[test]
    fn a_write_that_fails_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "a file where a directory has to be").unwrap();
        let path = blocker.join("generated.rs");

        let error = outcome(write_if_changed(&path, "x")).unwrap_err();

        assert!(error.contains(&path.display().to_string()), "{error}");
    }

    #[test]
    fn what_is_read_is_watched_from_the_manifest_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("app.toml"), "[app]").unwrap();
        let mut build = Build::new(dir.path().to_path_buf(), dir.path().join("out"));

        assert_eq!(outcome(build.read("app.toml")), Ok("[app]".to_string()));

        assert_eq!(build.tracked(), [dir.path().join("app.toml")]);
    }

    #[test]
    fn reading_what_is_not_there_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut build = Build::new(dir.path().to_path_buf(), dir.path().join("out"));

        let error = outcome(build.read("app.toml")).unwrap_err();

        let missing = dir.path().join("app.toml").display().to_string();
        assert!(error.contains(&missing), "{error}");
        assert_eq!(build.tracked(), [dir.path().join("app.toml")]);
    }

    #[test]
    fn what_is_tracked_resolves_against_the_manifest_dir_unless_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let mut build = Build::new(dir.path().to_path_buf(), dir.path().join("out"));

        assert_eq!(build.track("src"), dir.path().join("src"));
        assert_eq!(build.track(elsewhere.path()), elsewhere.path());

        assert_eq!(build.tracked(), [dir.path().join("src"), elsewhere.path().to_path_buf()]);
    }

    #[test]
    fn what_is_written_goes_to_the_out_dir() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let build = Build::new(dir.path().to_path_buf(), out.clone());

        assert_eq!(outcome(build.write("route_id.rs", "fn f() {}")), Ok(out.join("route_id.rs")));

        assert_eq!(fs::read_to_string(out.join("route_id.rs")).unwrap(), "fn f() {}");
        assert_eq!(build.tracked(), [] as [PathBuf; 0]);
    }
}
