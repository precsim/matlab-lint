use std::fs;
use std::path::{Path, PathBuf};
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

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout UTF-8")
        .trim()
        .to_owned()
}

fn init_repository(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "user.name", "mstyle test"]);
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

#[test]
fn disabled_lint_rule_is_filtered_from_cli_diagnostics() {
    let directory = temp_dir("lint-rule-off");
    let file = directory.join("sample.m");
    let config = directory.join("mstyle.toml");
    fs::write(&file, "x = 1; y = 2;\n").expect("write fixture");
    fs::write(&config, "[rules]\nL002 = \"off\"\n").expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .arg("--config")
        .arg(&config)
        .args(["lint", "--format", "json"])
        .arg(&file)
        .output()
        .expect("run lint");

    assert!(
        output.status.success(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON diagnostics");
    assert_eq!(json["ok"], true);
    assert_eq!(
        json["diagnostics"].as_array().expect("diagnostics").len(),
        0
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn missing_semicolon_is_off_by_default_and_can_be_enabled() {
    let directory = temp_dir("missing-semicolon");
    let file = directory.join("sample.m");
    let config = directory.join("mstyle.toml");
    fs::write(&file, "function y = sample()\n  y = 1\nend\n").expect("write fixture");

    let default_output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["lint", "--format", "json"])
        .arg(&file)
        .output()
        .expect("run default lint");
    assert!(default_output.status.success());
    let default_json: serde_json::Value =
        serde_json::from_slice(&default_output.stdout).expect("valid default JSON diagnostics");
    assert_eq!(default_json["ok"], true);

    fs::write(&config, "[rules]\nL010 = \"warning\"\n").expect("write config");
    let enabled_output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .arg("--config")
        .arg(&config)
        .args(["lint", "--format", "json"])
        .arg(&file)
        .output()
        .expect("run lint with L010 enabled");
    assert_eq!(enabled_output.status.code(), Some(1));
    let enabled_json: serde_json::Value =
        serde_json::from_slice(&enabled_output.stdout).expect("valid enabled JSON diagnostics");
    assert!(
        enabled_json["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|diagnostic| diagnostic["rule"] == "L010")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn lint_changed_limits_diagnostics_to_working_tree_changes() {
    let directory = temp_dir("lint-changed");
    init_repository(&directory);
    fs::write(directory.join("unchanged.m"), "a = 1; b = 2;\n").expect("unchanged source");
    fs::write(directory.join("changed.m"), "x = 1;\n").expect("changed source");
    git(&directory, &["add", "."]);
    git(&directory, &["commit", "-qm", "base"]);
    fs::write(directory.join("changed.m"), "x = 1; y = 2;\n").expect("modify source");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .current_dir(&directory)
        .args(["lint", "--changed", "--format", "json"])
        .output()
        .expect("run lint --changed");

    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON diagnostics");
    let diagnostics = json["diagnostics"].as_array().expect("diagnostics");
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["file"] == "changed.m")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn lint_diff_limits_diagnostics_to_branch_changes() {
    let directory = temp_dir("lint-diff");
    init_repository(&directory);
    fs::write(directory.join("unchanged.m"), "a = 1; b = 2;\n").expect("unchanged source");
    fs::write(directory.join("changed.m"), "x = 1;\n").expect("changed source");
    git(&directory, &["add", "."]);
    git(&directory, &["commit", "-qm", "base"]);
    let base = git(&directory, &["rev-parse", "HEAD"]);
    fs::write(directory.join("changed.m"), "x = 1; y = 2;\n").expect("modify source");
    git(&directory, &["add", "changed.m"]);
    git(&directory, &["commit", "-qm", "lint finding"]);

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .current_dir(&directory)
        .args(["lint", "--diff", &base, "--format", "json"])
        .output()
        .expect("run lint --diff");

    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON diagnostics");
    let diagnostics = json["diagnostics"].as_array().expect("diagnostics");
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["file"] == "changed.m")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}
