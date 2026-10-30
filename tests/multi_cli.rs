use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rs-fn-fingerprint"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).unwrap()
}

fn value(cwd: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&success(run(cwd, args))).unwrap()
}

#[test]
fn mixed_groups_equal_single_modes_and_keep_order_and_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    fs::write(cwd.join("code.rs"), "fn first() {} fn second() {}").unwrap();
    fs::write(cwd.join("data.json"), r#"{"rows":[{"id":42,"v":3}]}"#).unwrap();
    fs::write(cwd.join("data.toml"), "[settings]\nx=1\n[extras]\nx=2\n").unwrap();
    fs::write(cwd.join("raw"), [0, 255, 10, 13]).unwrap();
    let output = value(
        cwd,
        &[
            "multi",
            "--rust",
            "code.rs",
            "same=fn:first",
            "--json",
            "data.json",
            "row=field:id:42",
            "path:rows.*",
            "--file",
            "raw",
            "--toml",
            "data.toml",
            "config=settings",
            "all=*",
            "--rust",
            "./code.rs",
            "same=fn:second",
            "--toml",
            "data.toml",
            "same=extras",
            "--json",
            "data.json",
            "same=path:rows",
        ],
    );
    let cases: &[(&str, &str, &[&str])] = &[
        ("rust", "code.rs", &["same=fn:first"]),
        ("json", "data.json", &["row=field:id:42", "path:rows.*"]),
        ("file", "raw", &[]),
        ("toml", "data.toml", &["config=settings", "all=*"]),
        ("rust", "./code.rs", &["same=fn:second"]),
        ("toml", "data.toml", &["same=extras"]),
        ("json", "data.json", &["same=path:rows"]),
    ];
    assert_eq!(output["groups"].as_array().unwrap().len(), cases.len());
    for (i, (mode, file, queries)) in cases.iter().enumerate() {
        let mut args = vec![*mode, *file];
        args.extend_from_slice(queries);
        assert_eq!(
            output["groups"][i],
            json!({
                "mode": mode, "file": file, "results": value(cwd, &args),
            })
        );
    }
    assert_eq!(
        output["groups"][2]["results"]["bytes"],
        format!("sha256:{:x}", Sha256::digest([0, 255, 10, 13]))
    );
}

#[test]
fn combined_digest_covers_order_content_mode_path_and_group_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    fs::write(cwd.join("data.toml"), "[a]\nx=1\n[b]\nx=2\n").unwrap();
    fs::write(
        cwd.join("copy.toml"),
        fs::read(cwd.join("data.toml")).unwrap(),
    )
    .unwrap();
    let digest = |args: &[&str]| success(run(cwd, args));
    let original = digest(&[
        "multi",
        "--combine",
        "--toml",
        "data.toml",
        "a",
        "--file",
        "data.toml",
    ]);
    assert!(original.starts_with("sha256:"));
    assert_eq!(original.len(), 72);
    for args in [
        vec![
            "multi",
            "--file",
            "data.toml",
            "--toml",
            "data.toml",
            "a",
            "--combine",
        ],
        vec![
            "multi",
            "--toml",
            "copy.toml",
            "a",
            "--file",
            "data.toml",
            "--combine",
        ],
        vec![
            "multi",
            "--toml",
            "./data.toml",
            "a",
            "--file",
            "data.toml",
            "--combine",
        ],
        vec![
            "multi",
            "--toml",
            "data.toml",
            "b",
            "--file",
            "data.toml",
            "--combine",
        ],
        vec![
            "multi",
            "--toml",
            "data.toml",
            "a",
            "--toml",
            "data.toml",
            "a",
            "--combine",
        ],
    ] {
        assert_ne!(original, digest(&args));
    }
    let together = digest(&["multi", "--combine", "--toml", "data.toml", "a", "b"]);
    assert_ne!(
        together,
        digest(&[
            "multi",
            "--combine",
            "--toml",
            "data.toml",
            "a",
            "--toml",
            "data.toml",
            "b"
        ])
    );
    assert_eq!(
        together,
        digest(&["--combine", "multi", "--toml", "data.toml", "b", "a"])
    );
    fs::write(cwd.join("data.toml"), "[a]\nx=9\n[b]\nx=2\n").unwrap();
    assert_ne!(
        original,
        digest(&[
            "multi",
            "--combine",
            "--toml",
            "data.toml",
            "a",
            "--file",
            "data.toml"
        ])
    );
}

#[test]
fn missing_files_are_explicit_empty_inputs_and_rust_requires_allow_missing() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let args = [
        "multi",
        "--toml",
        "missing.toml",
        "x=*",
        "--json",
        "missing.json",
        "x=path:*",
        "--file",
        "missing.raw",
    ];
    let missing = value(cwd, &args);
    fs::write(cwd.join("missing.toml"), "").unwrap();
    fs::write(cwd.join("missing.json"), "{}").unwrap();
    fs::write(cwd.join("missing.raw"), "").unwrap();
    assert_eq!(missing, value(cwd, &args));
    assert_eq!(
        missing["groups"][0]["results"],
        value(cwd, &["toml", "absent.toml", "x=*"])
    );
    assert_eq!(
        missing["groups"][1]["results"],
        value(cwd, &["json", "absent.json", "x=path:*"])
    );
    assert_eq!(
        missing["groups"][2]["results"],
        value(cwd, &["file", "absent.raw"])
    );
    for args in [
        vec!["multi", "--rust", "absent.rs", "x=fn:*"],
        vec!["rust", "absent.rs", "x=fn:*"],
    ] {
        let output = run(cwd, &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    for args in [
        vec!["--allow-missing", "multi", "--rust", "absent.rs", "x=fn:*"],
        vec!["multi", "--rust", "absent.rs", "x=fn:*", "--allow-missing"],
    ] {
        assert_eq!(value(cwd, &args)["groups"][0]["results"], json!({"x":null}));
    }
    assert_eq!(
        value(cwd, &["rust", "absent.rs", "x=fn:*", "--allow-missing"]),
        json!({"x":null})
    );
    fs::write(cwd.join("code.rs"), "fn f() {}").unwrap();
    assert_eq!(
        value(
            cwd,
            &[
                "multi",
                "--rust",
                "code.rs",
                "fn:missing",
                "--allow-missing"
            ]
        )["groups"][0]["results"],
        json!({"fn:missing":null})
    );
    fs::write(cwd.join("missing.toml"), "[present]\nx=1").unwrap();
    fs::write(cwd.join("missing.json"), r#"{"present":1}"#).unwrap();
    fs::write(cwd.join("missing.raw"), "present").unwrap();
    let present = value(cwd, &args);
    for i in 0..3 {
        assert_ne!(missing["groups"][i], present["groups"][i]);
    }
}

#[test]
fn invalid_groups_sources_queries_and_io_fail_without_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    fs::write(cwd.join("valid.rs"), "fn f() {}").unwrap();
    fs::write(cwd.join("bad.rs"), "fn {").unwrap();
    fs::write(cwd.join("bad.json"), "").unwrap();
    fs::write(cwd.join("bad.toml"), "[").unwrap();
    fs::write(cwd.join("binary"), [255]).unwrap();
    for args in [
        vec!["multi"],
        vec!["multi", "--combine"],
        vec!["multi", "--rust"],
        vec!["multi", "--rust", "valid.rs"],
        vec!["multi", "--file", "valid.rs", "extra-query"],
        vec!["multi", "--unknown", "valid.rs"],
        vec!["multi", "rust", "valid.rs", "fn:f"],
        vec!["multi", "--rust", "--json", "file.json", "path:*"],
        vec![
            "multi", "--rust", "valid.rs", "fn:f", "--rust", "bad.rs", "fn:*",
        ],
        vec!["multi", "--json", "bad.json", "path:*"],
        vec!["multi", "--toml", "bad.toml", "*"],
        vec!["multi", "--toml", "binary", "*"],
        vec!["multi", "--file", "."],
        vec!["multi", "--rust", "valid.rs", "invalid"],
        vec!["multi", "--rust", "valid.rs", "fn:missing"],
        vec![
            "multi",
            "--rust",
            "valid.rs",
            "same=fn:f",
            "same=fn:missing",
            "--allow-missing",
        ],
        vec![
            "multi",
            "--json",
            "absent.json",
            "invalid",
            "--allow-missing",
        ],
    ] {
        let output = run(cwd, &args);
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty());
    }
    assert!(run(cwd, &["multi", "--help"]).status.success());
    assert!(run(cwd, &["file", "--help"]).status.success());
}

#[test]
fn existing_single_mode_digest_vectors_do_not_change() {
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        success(run(
            cwd,
            &[
                "rust",
                "tests/fixtures/items.rs",
                "fn:separate",
                "--combine"
            ]
        )),
        "sha256:9898927c0217cd14608bb55117415b35bb1b98e009f1d641c23b4fd2ad6d91b0\n"
    );
    assert_eq!(
        success(run(
            cwd,
            &[
                "toml",
                "tests/fixtures/tables.toml",
                "settings.100",
                "--combine"
            ]
        )),
        "sha256:9e56ac8bbd6a6c734d03685cc02eeb3c9cebe4037accbcff18032cc2ac260090\n"
    );
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("data.json"),
        r#"{"rows":[{"id":42,"v":3}]}"#,
    )
    .unwrap();
    assert_eq!(
        success(run(
            dir.path(),
            &["json", "data.json", "field:id:42", "--combine"]
        )),
        "sha256:db88557ad323b27fa88feada9f682e7d4624c1688d179a37fad6c965f10e8aff\n"
    );
}
