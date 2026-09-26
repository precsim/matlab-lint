use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("mstyle-{name}-{}-{sequence}", std::process::id()));
    fs::create_dir_all(&path).expect("create temp directory");
    path
}

#[test]
fn lint_json_has_stable_schema_and_source_exit_code() {
    let directory = temp_dir("lint-json");
    let file = directory.join("sample.m");
    let config = directory.join("mstyle.toml");
    fs::write(&file, "x = 1; y = 2;\n").expect("write fixture");
    fs::write(
        &config,
        "[lint]\nline_length = 120\n[rules]\nL002 = \"error\"\n",
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .arg("--config")
        .arg(&config)
        .args(["lint", "--format", "json"])
        .arg(&file)
        .output()
        .expect("run lint");

    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON diagnostics");
    assert_eq!(json["ok"], false);
    assert_eq!(json["diagnostics"][0]["rule"], "L002");
    assert_eq!(json["diagnostics"][0]["severity"], "error");
    assert_eq!(json["diagnostics"][0]["fixable"], false);

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn check_aggregates_formatter_and_lint_diagnostics_without_writing() {
    let directory = temp_dir("check");
    let file = directory.join("sample.m");
    fs::write(&file, "x=1; y=2;").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["check", "--format", "json"])
        .arg(&file)
        .output()
        .expect("run check");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&file).expect("read"), "x=1; y=2;");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON diagnostics");
    let rules: Vec<_> = json["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .map(|diagnostic| diagnostic["rule"].as_str().expect("rule"))
        .collect();

    assert!(rules.contains(&"F002"));
    assert!(rules.contains(&"F007"));
    assert!(rules.contains(&"L002"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}
