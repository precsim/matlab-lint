use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use mstyle::discovery::{discover_changed_from, discover_diff_from};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mstyle-git-{name}-{}-{sequence}",
        std::process::id()
    ));
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
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

#[test]
fn diff_discovery_includes_tracked_renamed_and_untracked_but_not_deleted_or_ignored() {
    let root = temp_dir("diff");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "mstyle test"]);

    fs::write(root.join("tracked.m"), "x = 1;\n").expect("tracked");
    fs::write(root.join("old.m"), "y = 1;\n").expect("old");
    fs::write(root.join("deleted.m"), "z = 1;\n").expect("deleted");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);

    fs::write(root.join("tracked.m"), "x = 2;\n").expect("modify");
    git(&root, &["mv", "old.m", "renamed.m"]);
    fs::remove_file(root.join("deleted.m")).expect("delete");
    fs::write(root.join("untracked.m"), "u = 1;\n").expect("untracked");
    fs::write(root.join("ignored.m"), "i = 1;\n").expect("ignored");
    fs::write(root.join(".gitignore"), "ignored.m\n").expect("gitignore");

    let targets = discover_diff_from(&root, &base, &[]).expect("discover diff");
    let paths: Vec<_> = targets
        .iter()
        .map(|target| target.display_path.to_string_lossy().into_owned())
        .collect();

    assert_eq!(paths, vec!["renamed.m", "tracked.m", "untracked.m"]);

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn changed_discovery_includes_staged_unstaged_and_untracked_only() {
    let root = temp_dir("changed");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "mstyle test"]);

    fs::write(root.join("staged.m"), "x = 1;\n").expect("staged");
    fs::write(root.join("unstaged.m"), "y = 1;\n").expect("unstaged");
    fs::write(root.join("clean.m"), "z = 1;\n").expect("clean");
    fs::write(root.join("deleted.m"), "d = 1;\n").expect("deleted");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);

    fs::write(root.join("staged.m"), "x = 2;\n").expect("modify staged");
    git(&root, &["add", "staged.m"]);
    fs::write(root.join("unstaged.m"), "y = 2;\n").expect("modify unstaged");
    fs::remove_file(root.join("deleted.m")).expect("delete");
    fs::write(root.join("untracked.m"), "u = 1;\n").expect("untracked");
    fs::write(root.join("ignored.m"), "i = 1;\n").expect("ignored");
    fs::write(root.join(".gitignore"), "ignored.m\n").expect("gitignore");

    let targets = discover_changed_from(&root, &[]).expect("discover changed");
    let paths: Vec<_> = targets
        .iter()
        .map(|target| target.display_path.to_string_lossy().into_owned())
        .collect();

    assert_eq!(paths, vec!["staged.m", "unstaged.m", "untracked.m"]);

    fs::remove_dir_all(root).expect("cleanup");
}
