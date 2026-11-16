use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

#[test]
fn fmt_check_is_non_mutating_and_fmt_writes_transactionally() {
    let directory = temp_dir("fmt");
    let file = directory.join("sample.m");
    fs::write(&file, "x=1 ;").expect("write fixture");

    let check = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["fmt", "--check"])
        .arg(&file)
        .output()
        .expect("run fmt --check");

    assert_eq!(check.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&file).expect("read input"), "x=1 ;");
    assert!(
        String::from_utf8_lossy(&check.stderr).contains("needs formatting"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stderr).contains("F007"));

    let format = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .arg("fmt")
        .arg(&file)
        .output()
        .expect("run fmt");

    assert!(
        format.status.success(),
        "fmt failed: {}",
        String::from_utf8_lossy(&format.stderr)
    );
    assert_eq!(
        fs::read_to_string(&file).expect("read formatted source"),
        "x = 1;\n"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn fmt_check_json_reports_formatter_rules() {
    let directory = temp_dir("fmt-check-json");
    let file = directory.join("sample.m");
    fs::write(&file, "x=1 ;").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["fmt", "--check", "--format", "json"])
        .arg(&file)
        .output()
        .expect("run fmt --check --format json");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&file).expect("read input"), "x=1 ;");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON diagnostics");
    let diagnostics = json["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(json["ok"], false);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["rule"] == "F007")
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["fixable"] == true)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn disabled_formatter_rule_is_not_applied_or_reported() {
    let directory = temp_dir("fmt-rule-off");
    let file = directory.join("sample.m");
    let config = directory.join("mstyle.toml");
    fs::write(&file, "x=1;\n").expect("write fixture");
    fs::write(&config, "[rules]\nF007 = \"off\"\n").expect("write config");

    let check = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .arg("--config")
        .arg(&config)
        .args(["fmt", "--check", "--format", "json"])
        .arg(&file)
        .output()
        .expect("run fmt check");
    assert!(
        check.status.success(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&check.stdout).expect("valid JSON diagnostics");
    assert_eq!(json["ok"], true);
    assert_eq!(
        json["diagnostics"].as_array().expect("diagnostics").len(),
        0
    );

    let format = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .arg("--config")
        .arg(&config)
        .arg("fmt")
        .arg(&file)
        .output()
        .expect("run fmt");
    assert!(format.status.success());
    assert_eq!(fs::read_to_string(&file).expect("read source"), "x=1;\n");

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn fmt_check_changed_limits_format_diagnostics_to_working_tree_changes() {
    let directory = temp_dir("fmt-changed");
    git(&directory, &["init", "-q"]);
    git(
        &directory,
        &["config", "user.email", "test@example.invalid"],
    );
    git(&directory, &["config", "user.name", "mstyle test"]);
    fs::write(directory.join("unchanged.m"), "x=1;\n").expect("unchanged source");
    fs::write(directory.join("changed.m"), "y = 1;\n").expect("changed source");
    git(&directory, &["add", "."]);
    git(&directory, &["commit", "-qm", "base"]);
    fs::write(directory.join("changed.m"), "y=1;\n").expect("modify source");

    let output = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .current_dir(&directory)
        .args(["fmt", "--check", "--changed", "--format", "json"])
        .output()
        .expect("run fmt --check --changed");

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
fn malformed_fmt_applies_only_safe_cleanup_and_returns_source_status() {
    let directory = temp_dir("malformed");
    let file = directory.join("broken.m");
    fs::write(&file, "x = 1 + ;   ").expect("write fixture");

    let format = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .arg("fmt")
        .arg(&file)
        .output()
        .expect("run fmt");

    assert_eq!(format.status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(&file).expect("read formatted source"),
        "x = 1 + ;\n"
    );
    assert!(
        String::from_utf8_lossy(&format.stderr).contains("PARSE"),
        "expected parse diagnostic: {}",
        String::from_utf8_lossy(&format.stderr)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn fmt_stdin_writes_formatted_source_to_stdout() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["fmt", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn fmt --stdin");

    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(b"x=1 ;")
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for fmt --stdin");

    assert!(
        output.status.success(),
        "fmt --stdin failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout UTF-8"),
        "x = 1;\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn malformed_fmt_stdin_applies_only_safe_cleanup_and_returns_source_status() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .args(["fmt", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn fmt --stdin");

    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(b"x = 1 + ;   ")
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for fmt --stdin");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout UTF-8"),
        "x = 1 + ;\n"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("<stdin>:1:"),
        "expected parse diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
