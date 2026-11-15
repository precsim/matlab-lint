use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use mstyle::config::DiffVcs;
use mstyle::config::DiffConfig;
use mstyle::diff::analyze_diff;
use mstyle::discovery::{
    ChangeKind, ChangedFile, ChangedWorkspace, discover_working_tree_changes,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mstyle-diff-vcs-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create temp directory");
    path
}

fn run(root: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .current_dir(root)
        .args(args)
        .env("FOSSIL_USER", "mstyle-test")
        .output()
        .unwrap_or_else(|error| panic!("run {program} {args:?}: {error}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn fossil_available() -> bool {
    Command::new("fossil")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn write_base_files(root: &Path) {
    fs::write(root.join("modified.m"), "x = 1;\n").expect("modified base");
    fs::write(root.join("old.m"), "y = 1;\n").expect("rename base");
    fs::write(root.join("deleted.m"), "z = 1;\n").expect("delete base");
    fs::write(root.join("ignored.txt"), "not matlab\n").expect("non-matlab base");
    fs::write(root.join("ignored.bin"), [0xff, 0x00, 0xfe]).expect("binary base");
    fs::write(root.join("to_text.m"), "to_text = 1;\n").expect("m-to-text base");
    fs::write(root.join("to_matlab.txt"), "to_matlab = 1;\n").expect("text-to-m base");
}

fn make_working_changes(root: &Path) {
    fs::write(root.join("modified.m"), "x = 2;\n").expect("modify");
    fs::write(root.join("added.m"), "a = 1;\n").expect("managed add");
    fs::write(root.join("unmanaged.m"), "u = 1;\n").expect("unmanaged add");
    fs::write(root.join("ignored.txt"), "still not matlab\n").expect("non-matlab edit");
    fs::write(root.join("ignored.bin"), [0xfe, 0x00, 0xff]).expect("binary edit");
}

fn git_workspace() -> (PathBuf, Vec<ChangedFile>) {
    let root = temp_dir("git");
    run(&root, "git", &["init", "-q"]);
    run(
        &root,
        "git",
        &["config", "user.email", "test@example.invalid"],
    );
    run(&root, "git", &["config", "user.name", "mstyle test"]);
    write_base_files(&root);
    run(&root, "git", &["add", "."]);
    run(&root, "git", &["commit", "-qm", "base"]);

    make_working_changes(&root);
    run(&root, "git", &["add", "added.m"]);
    run(&root, "git", &["mv", "old.m", "renamed.m"]);
    run(&root, "git", &["mv", "to_text.m", "to_text.txt"]);
    run(&root, "git", &["mv", "to_matlab.txt", "to_matlab.m"]);
    fs::remove_file(root.join("deleted.m")).expect("delete file");

    let workspace =
        discover_working_tree_changes(&root, DiffVcs::Git).expect("discover Git changes");
    (root, workspace.files)
}

fn fossil_workspace() -> (PathBuf, Vec<ChangedFile>) {
    let parent = temp_dir("fossil-parent");
    let repository = parent.join("repo.fossil");
    let checkout = parent.join("checkout");
    fs::create_dir_all(&checkout).expect("create checkout");
    let repository_text = repository.to_string_lossy().into_owned();

    run(&parent, "fossil", &["init", &repository_text]);
    run(&checkout, "fossil", &["open", &repository_text]);
    run(&checkout, "fossil", &["settings", "autosync", "off"]);
    write_base_files(&checkout);
    run(&checkout, "fossil", &["add", "."]);
    run(
        &checkout,
        "fossil",
        &["commit", "--nosync", "--no-prompt", "-m", "base"],
    );

    make_working_changes(&checkout);
    run(&checkout, "fossil", &["add", "added.m"]);
    run(
        &checkout,
        "fossil",
        &["mv", "--hard", "old.m", "renamed.m"],
    );
    run(
        &checkout,
        "fossil",
        &["mv", "--hard", "to_text.m", "to_text.txt"],
    );
    run(
        &checkout,
        "fossil",
        &["mv", "--hard", "to_matlab.txt", "to_matlab.m"],
    );
    run(
        &checkout,
        "fossil",
        &["rm", "--hard", "deleted.m"],
    );

    let workspace =
        discover_working_tree_changes(&checkout, DiffVcs::Fossil).expect("discover Fossil changes");
    (parent, workspace.files)
}

#[derive(Debug, PartialEq, Eq)]
struct Summary {
    kind: ChangeKind,
    old_path: Option<String>,
    new_path: Option<String>,
    old_source: Option<String>,
    new_source: Option<String>,
}

fn summaries(files: &[ChangedFile]) -> Vec<Summary> {
    let mut summaries = files
        .iter()
        .map(|file| Summary {
            kind: file.kind,
            old_path: file
                .old_path
                .as_ref()
                .map(|path| path.to_string_lossy().replace('\\', "/")),
            new_path: file
                .new_path
                .as_ref()
                .map(|path| path.to_string_lossy().replace('\\', "/")),
            old_source: file.old_source.clone(),
            new_source: file.new_source.clone(),
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        left.new_path
            .as_ref()
            .or(left.old_path.as_ref())
            .cmp(&right.new_path.as_ref().or(right.old_path.as_ref()))
    });
    summaries
}

#[test]
fn git_working_tree_provider_covers_all_change_kinds_and_ignores_non_matlab() {
    let (root, files) = git_workspace();
    let summary = summaries(&files);

    assert_eq!(summary.len(), 7);
    assert!(summary.iter().any(|file| {
        file.kind == ChangeKind::Modified
            && file.new_path.as_deref() == Some("modified.m")
            && file.old_source.as_deref() == Some("x = 1;\n")
            && file.new_source.as_deref() == Some("x = 2;\n")
    }));
    assert!(summary.iter().any(|file| {
        file.kind == ChangeKind::Renamed
            && file.old_path.as_deref() == Some("old.m")
            && file.new_path.as_deref() == Some("renamed.m")
    }));
    assert!(summary.iter().any(|file| {
        file.kind == ChangeKind::Deleted
            && file.old_path.as_deref() == Some("deleted.m")
            && file.new_source.is_none()
    }));
    assert_eq!(
        summary
            .iter()
            .filter(|file| file.kind == ChangeKind::Added)
            .count(),
        3
    );
    assert!(summary.iter().any(|file| {
        file.kind == ChangeKind::Deleted
            && file.old_path.as_deref() == Some("to_text.m")
            && file.new_path.is_none()
    }));
    assert!(summary.iter().any(|file| {
        file.kind == ChangeKind::Added
            && file.old_path.is_none()
            && file.new_path.as_deref() == Some("to_matlab.m")
    }));

    fs::remove_dir_all(root).expect("cleanup Git repository");
}

#[test]
fn git_and_fossil_providers_produce_equivalent_changed_file_models() {
    if !fossil_available() {
        eprintln!("skipping Fossil provider test because fossil is unavailable");
        return;
    }

    let (git_root, git_files) = git_workspace();
    let (fossil_parent, fossil_files) = fossil_workspace();

    assert_eq!(summaries(&git_files), summaries(&fossil_files));
    let git_analysis = analyze_diff(
        &ChangedWorkspace {
            root: git_root.clone(),
            vcs: mstyle::discovery::DetectedVcs::Git,
            files: git_files.clone(),
        },
        &DiffConfig::default(),
    )
    .expect("analyze Git changes");
    let fossil_checkout = fossil_parent.join("checkout");
    let fossil_analysis = analyze_diff(
        &ChangedWorkspace {
            root: fossil_checkout,
            vcs: mstyle::discovery::DetectedVcs::Fossil,
            files: fossil_files.clone(),
        },
        &DiffConfig::default(),
    )
    .expect("analyze Fossil changes");
    assert_eq!(git_analysis.metrics, fossil_analysis.metrics);
    assert_eq!(git_analysis.test_metrics, fossil_analysis.test_metrics);
    assert_eq!(git_analysis.decision, fossil_analysis.decision);
    assert_eq!(git_analysis.files, fossil_analysis.files);

    fs::remove_dir_all(git_root).expect("cleanup Git repository");
    fs::remove_dir_all(fossil_parent).expect("cleanup Fossil repository");
}
