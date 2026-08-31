//! The rules the interface bundle has to keep, checked mechanically.
//!
//! Cryptography does not survive script running on the page that renders the secrets, and it does
//! not survive a key written somewhere that outlives the lock. Both failures are one careless line
//! away and neither shows up in a screenshot, so they are tests rather than a paragraph in a
//! contributing guide.
//!
//! These run against the source, not the built bundle: the point is to fail on the pull request
//! that introduces the line, naming the file.

use std::fs;
use std::path::{Path, PathBuf};

/// Patterns that must not appear anywhere in the interface source, and why each one is here.
const FORBIDDEN: &[(&str, &str)] = &[
    ("dangerouslySetInnerHTML", "renders markup as HTML — this is how a stored value becomes script"),
    ("innerHTML", "same, one abstraction lower"),
    ("outerHTML", "same"),
    ("insertAdjacentHTML", "same"),
    ("eval(", "executes a string on the page that holds the keys"),
    ("new Function(", "eval with a different spelling"),
    ("localStorage", "a key that survives the lock is a vault that never locked"),
    ("sessionStorage", "same, with a shorter lifetime and the same problem"),
    ("indexedDB", "same, with more room"),
    ("document.cookie", "storage the browser attaches to requests on its own"),
    ("http://", "no runtime fetch: everything the page needs is in the binary"),
    ("https://", "no runtime fetch: everything the page needs is in the binary"),
];

/// A comment line, in any of the languages the interface is written in.
///
/// Comments are skipped because explaining *why* something is forbidden requires naming it, and a
/// codebase that cannot write its own reasons down loses them. The rule stays conservative: only a
/// line that begins as a comment is skipped, never the tail of a line that starts as code, so
/// `foo(localStorage) // fine, honest` is still caught.
fn is_comment(line: &str) -> bool {
    let trimmed = line.trim_start();

    trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*')
}

/// Files whose whole job is to name these patterns.
fn is_exempt(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.ends_with(".test.ts") || name.ends_with(".test.tsx"))
        .unwrap_or(false)
}

fn web_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                walk(&path, found);
                continue;
            }

            let is_source = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| matches!(ext, "ts" | "tsx" | "css" | "html"))
                .unwrap_or(false);

            if is_source && !is_exempt(&path) {
                found.push(path);
            }
        }
    }

    let mut found = Vec::new();
    walk(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/src"), &mut found);
    found
}

#[test]
fn the_interface_source_is_actually_there() {
    // Without this, every check below would pass by finding nothing.
    assert!(
        !web_sources().is_empty(),
        "no interface source found — the checks below would pass vacuously"
    );
}

#[test]
fn the_interface_carries_none_of_the_forbidden_patterns() {
    let mut violations = Vec::new();

    for path in web_sources() {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };

        for (line_no, line) in contents.lines().enumerate() {
            if is_comment(line) {
                continue;
            }

            for (pattern, reason) in FORBIDDEN {
                if line.contains(pattern) {
                    violations.push(format!(
                        "{}:{} — `{}` ({})",
                        path.display(),
                        line_no + 1,
                        pattern,
                        reason
                    ));
                }
            }
        }
    }

    assert!(violations.is_empty(), "forbidden in the vault interface:\n{}", violations.join("\n"));
}

#[test]
fn the_bundle_fetches_nothing_at_runtime() {
    let html = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/index.html"),
    )
    .expect("web/index.html");

    for remote in ["http://", "https://", "//cdn", "//fonts."] {
        assert!(
            !html.contains(remote),
            "the document reaches out to `{remote}` — the binary must carry everything it renders"
        );
    }
}
