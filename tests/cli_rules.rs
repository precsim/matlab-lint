use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mstyle-cli-rules-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create test directory");
    path
}

#[test]
fn disabled_formatter_rules_are_not_applied_or_reported() {
    let root = temp_dir();
    let source = root.join("sample.m");
    let config = root.join("mstyle.toml");
    fs::write(&source, "x=1;\n").expect("write MATLAB source");
    fs::write(&config, "[rules]\nF007 = \"off\"\n").expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["--config"])
        .arg(&config)
        .args(["fmt", "--check", "--format", "json"])
        .arg(&source)
        .output()
        .expect("run formatter check");

    assert!(
        output.status.success(),
        "unexpected formatter findings: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(json["ok"], true);
    assert_eq!(json["diagnostics"].as_array().map(Vec::len), Some(0));

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["--config"])
        .arg(&config)
        .arg("fmt")
        .arg(&source)
        .output()
        .expect("run formatter");
    assert!(output.status.success());
    assert_eq!(fs::read_to_string(&source).expect("read source"), "x=1;\n");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn disabled_lint_rules_are_not_reported() {
    let root = temp_dir();
    let source = root.join("sample.m");
    let config = root.join("mstyle.toml");
    fs::write(&source, "x = 1; y = 2;\n").expect("write MATLAB source");
    fs::write(&config, "[rules]\nL002 = \"off\"\n").expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["--config"])
        .arg(&config)
        .args(["lint", "--format", "json"])
        .arg(&source)
        .output()
        .expect("run linter");
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(json["ok"], true);

    fs::remove_dir_all(root).expect("cleanup");
}
