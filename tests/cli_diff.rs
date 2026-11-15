use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mstyle-cli-diff-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create temp directory");
    path
}

fn git(root: &Path, args: &[&str]) {
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
}

fn repository(name: &str, base: &str) -> PathBuf {
    let root = temp_dir(name);
    git(&root, &["init", "-q"]);
    git(
        &root,
        &["config", "user.email", "test@example.invalid"],
    );
    git(&root, &["config", "user.name", "mstyle test"]);
    fs::write(root.join("sample.m"), base).expect("write base");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    root
}

fn diff_check(root: &Path, config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mstyle"))
        .current_dir(root)
        .arg("--config")
        .arg(config)
        .arg("diff-check")
        .args(args)
        .output()
        .expect("run diff-check")
}

#[test]
fn advisory_review_reports_policy_but_exits_successfully() {
    let root = repository("advisory", "x = 1;\n");
    fs::write(root.join("sample.m"), "x = 2;\n").expect("modify source");
    let config = root.join("mstyle.toml");
    fs::write(
        &config,
        concat!(
            "[diff]\n",
            "advisory = true\n",
            "[diff.thresholds.general]\n",
            "added_lines = { review = 1, reject = 10 }\n",
        ),
    )
    .expect("write config");

    let output = diff_check(&root, &config, &["--format", "json"]);
    assert!(
        output.status.success(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(json["decision"], "REVIEW");
    assert_eq!(json["advisory"], true);
    assert_eq!(json["metrics"]["added_executable_lines"], 1);
    assert!(json["triggers"].as_array().expect("triggers").iter().any(
        |trigger| trigger["id"]
            .as_str()
            .is_some_and(|id| id.contains("added_lines"))
    ));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn enforced_review_exits_with_source_finding_status() {
    let root = repository("enforced", "x = 1;\n");
    fs::write(root.join("sample.m"), "x = 2;\n").expect("modify source");
    let config = root.join("mstyle.toml");
    fs::write(
        &config,
        concat!(
            "[diff]\n",
            "advisory = false\n",
            "[diff.thresholds.general]\n",
            "added_lines = { review = 1, reject = 10 }\n",
        ),
    )
    .expect("write config");

    let output = diff_check(&root, &config, &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(json["decision"], "REVIEW");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn changed_parse_recovery_is_exit_one_even_in_advisory_mode() {
    let root = repository("parse", "x = 1;\n");
    fs::write(root.join("sample.m"), "x = 1 + ;\n").expect("break source");
    let config = root.join("mstyle.toml");
    fs::write(&config, "[diff]\nadvisory = true\n").expect("write config");

    let output = diff_check(&root, &config, &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(json["decision"], "REVIEW");
    assert_eq!(json["source_diagnostics"], true);

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn text_report_exposes_raw_metrics_and_decision() {
    let root = repository("text", "x = 1;\n");
    fs::write(root.join("sample.m"), "x = 2;\n").expect("modify source");
    let config = root.join("mstyle.toml");
    fs::write(&config, "[diff]\nadvisory = true\n").expect("write config");

    let output = diff_check(&root, &config, &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("text stdout");
    assert!(stdout.contains("decision: ACCEPT"));
    assert!(stdout.contains("executable lines added: 1"));
    assert!(stdout.contains("executable lines deleted: 1"));
    assert!(stdout.contains(".m files touched: 1"));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn text_and_json_reports_share_stable_fact_names() {
    let root = repository("parity", "x = A(1);\n");
    fs::write(root.join("sample.m"), "x = A(2);\n").expect("modify source");
    let config = root.join("mstyle.toml");
    fs::write(&config, "[diff]\nadvisory = true\n").expect("write config");

    let json_output = diff_check(&root, &config, &["--format", "json"]);
    assert!(json_output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&json_output.stdout).expect("valid JSON report");
    let fact_names = json["files"][0]["facts"]
        .as_array()
        .expect("facts")
        .iter()
        .filter_map(|fact| fact["kind"].as_str())
        .collect::<Vec<_>>();
    assert!(fact_names.contains(&"call_or_index_changed"));
    assert!(fact_names.iter().all(|name| {
        name.chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
    }));

    let text_output = diff_check(&root, &config, &[]);
    assert!(text_output.status.success());
    let text = String::from_utf8(text_output.stdout).expect("text report");
    assert!(text.contains(&format!(
        "decision: {}",
        json["decision"].as_str().expect("decision")
    )));
    for name in fact_names {
        assert!(text.contains(&format!("fact: {name}")));
    }

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn vcs_failure_is_exit_two() {
    let root = temp_dir("not-a-repository");
    let config = root.join("mstyle.toml");
    fs::write(&config, "").expect("write config");

    let output = diff_check(&root, &config, &["--vcs", "git"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("diff discovery error")
    );

    fs::remove_dir_all(root).expect("cleanup");
}
