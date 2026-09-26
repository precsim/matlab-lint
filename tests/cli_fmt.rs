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
