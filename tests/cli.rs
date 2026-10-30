use std::{
    path::PathBuf,
    process::{Command, Output},
};

use rs_fn_fingerprint::{Fingerprints, combine, fingerprint_rust, fingerprint_toml};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rs-fn-fingerprint"))
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Fingerprints {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn rust_and_toml_cli_match_library() {
    let path = fixture("items.rs");
    let queries = ["fn:separate", "arm:ops:1200"];
    let output = run(&["rust", path.to_str().unwrap(), queries[0], queries[1]]);
    assert_eq!(
        json(&output),
        fingerprint_rust(&std::fs::read_to_string(path).unwrap(), &queries, false).unwrap()
    );

    let path = fixture("tables.toml");
    let output = run(&[
        "toml",
        path.to_str().unwrap(),
        "config=settings.100",
        "absent.*",
    ]);
    assert_eq!(
        json(&output),
        fingerprint_toml(
            &std::fs::read_to_string(path).unwrap(),
            &["config=settings.100", "absent.*"]
        )
        .unwrap()
    );
}

#[test]
fn missing_queries_fail_atomically_unless_allowed() {
    let path = fixture("items.rs");
    let output = run(&[
        "rust",
        path.to_str().unwrap(),
        "fn:separate",
        "fn:no_such_item",
    ]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("matched no items"));

    let output = run(&[
        "rust",
        path.to_str().unwrap(),
        "fn:no_such_item",
        "--allow-missing",
    ]);
    assert_eq!(json(&output)["fn:no_such_item"], None);
}

#[test]
fn combine_is_a_plain_digest_with_a_newline_and_accepts_global_flags() {
    let path = fixture("items.rs");
    let expected = combine(
        &fingerprint_rust(
            &std::fs::read_to_string(&path).unwrap(),
            &["fn:separate"],
            false,
        )
        .unwrap(),
    );
    for args in [
        vec!["--combine", "rust", path.to_str().unwrap(), "fn:separate"],
        vec!["rust", path.to_str().unwrap(), "fn:separate", "--combine"],
    ] {
        let output = run(&args);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{expected}\n")
        );
    }
}

#[test]
fn parser_io_and_source_errors_never_write_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.rs");
    for source in ["fn {", "\u{feff}not rust"] {
        std::fs::write(&path, source).unwrap();
        let output = run(&["rust", path.to_str().unwrap(), "fn:*"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let missing = dir.path().join("missing.rs");
    let fixture = fixture("items.rs");
    for args in [
        vec!["rust", missing.to_str().unwrap(), "fn:*"],
        vec!["rust", fixture.to_str().unwrap()],
        vec!["rust", fixture.to_str().unwrap(), "not-a-query"],
        vec!["toml", fixture.to_str().unwrap(), "*"],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn version_and_help_work() {
    assert!(run(&["--version"]).status.success());
    assert!(run(&["--help"]).status.success());
    assert!(run(&["rust", "--help"]).status.success());
    assert!(run(&["toml", "--help"]).status.success());
}

#[test]
fn branch_and_json_cli_are_generic_and_fail_on_bad_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records.json");
    std::fs::write(&path, r#"{"nodes":{"42":{"id":42,"value":3}}}"#).unwrap();
    assert!(
        json(&run(&[
            "json",
            path.to_str().unwrap(),
            "field:id:42",
            "path:nodes.42"
        ]))
        .values()
        .all(Option::is_some)
    );
    assert!(
        !run(&["json", path.to_str().unwrap(), "invalid"])
            .status
            .success()
    );
    let rust = fixture("items.rs");
    assert!(
        json(&run(&["rust", rust.to_str().unwrap(), "branch:ops:999"]))
            .values()
            .all(Option::is_some)
    );
    assert!(run(&["json", "--help"]).status.success());
}
