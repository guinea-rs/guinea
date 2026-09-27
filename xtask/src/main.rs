//! `cargo xtask docs` copies the examples marked in tests into the pages that
//! show them: the README and the doc comments under `crates/*/src`.
//! `--check` writes nothing and fails when a page is behind its tests.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const KINDS: &[&str] = &["shown"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("docs") => match docs(args[1..].iter().any(|arg| arg == "--check")) {
            Ok(()) => ExitCode::SUCCESS,
            Err(problems) => {
                for problem in problems {
                    eprintln!("{problem}");
                }
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo xtask docs [--check]");
            ExitCode::FAILURE
        }
    }
}

fn docs(check: bool) -> Result<(), Vec<String>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the workspace root")
        .to_path_buf();

    let mut files = Vec::new();
    walk(&root.join("crates"), &mut files).map_err(|error| vec![error])?;

    let mut marks = shown::Marks::new();
    for test in files.iter().filter(|file| is_test(&root, file)) {
        let text = read(test)?;
        marks
            .read(relative(&root, test), &text)
            .map_err(|error| vec![error.to_string()])?;
    }

    let pages = std::iter::once(root.join("README.md"))
        .chain(files.iter().filter(|file| is_page(&root, file)).cloned());

    let mut problems = Vec::new();
    let mut used = BTreeSet::new();
    let mut behind = Vec::new();

    for page in pages {
        let text = read(&page)?;
        if !text.contains("<!-- shown:") {
            continue;
        }

        let name = relative(&root, &page);
        let fence = if page.extension().is_some_and(|ext| ext == "rs") {
            "rust,ignore"
        } else {
            "rust"
        };

        let filled = match shown::fill(name, &text, KINDS, |asked| {
            used.insert(asked.arg.clone());
            marks.block(&asked.arg, fence)
        }) {
            Ok(filled) => filled,
            Err(error) => {
                problems.push(error.to_string());
                continue;
            }
        };

        for asked in &filled.unanswered {
            problems.push(format!(
                "{}:{}: no test marks `{}`",
                name.display(),
                asked.line,
                asked.arg
            ));
        }

        if filled.text == text {
            continue;
        }

        if check {
            behind.push(name.display().to_string());
        } else if let Err(error) = fs::write(&page, &filled.text) {
            problems.push(format!("{}: {error}", name.display()));
        } else {
            eprintln!("filled {}", name.display());
        }
    }

    for region in marks.iter().filter(|region| !used.contains(&region.name)) {
        eprintln!(
            "{}:{}: `{}` is marked and no page shows it",
            region.file.display(),
            region.line,
            region.name
        );
    }

    if !behind.is_empty() {
        problems.push(format!(
            "behind their tests, run `cargo xtask docs`: {}",
            behind.join(", ")
        ));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn walk(dir: &Path, into: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;

    for entry in entries {
        let path = entry
            .map_err(|error| format!("{}: {error}", dir.display()))?
            .path();

        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                walk(&path, into)?;
            }
        } else if path
            .extension()
            .is_some_and(|ext| ext == "rs" || ext == "md")
        {
            into.push(path);
        }
    }

    Ok(())
}

fn is_test(root: &Path, file: &Path) -> bool {
    file.extension().is_some_and(|ext| ext == "rs") && under(root, file, "tests")
}

fn is_page(root: &Path, file: &Path) -> bool {
    let md = file.extension().is_some_and(|ext| ext == "md");
    md || under(root, file, "src")
}

fn under(root: &Path, file: &Path, dir: &str) -> bool {
    relative(root, file)
        .components()
        .any(|component| component.as_os_str() == dir)
}

fn relative<'a>(root: &Path, file: &'a Path) -> &'a Path {
    file.strip_prefix(root).unwrap_or(file)
}

fn read(file: &Path) -> Result<String, Vec<String>> {
    fs::read_to_string(file).map_err(|error| vec![format!("{}: {error}", file.display())])
}
