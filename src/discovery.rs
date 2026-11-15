use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::Serialize;

use crate::config::DiffVcs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
    pub io_path: PathBuf,
    pub display_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectedVcs {
    Git,
    Fossil,
}

impl fmt::Display for DetectedVcs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Git => "git",
            Self::Fossil => "fossil",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Modified => "modified",
            Self::Renamed => "renamed",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub kind: ChangeKind,
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub old_source: Option<String>,
    pub new_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedWorkspace {
    pub root: PathBuf,
    pub vcs: DetectedVcs,
    pub files: Vec<ChangedFile>,
}

pub fn discover_paths(
    inputs: &[PathBuf],
    exclude_patterns: &[String],
) -> Result<Vec<FileTarget>, DiscoveryError> {
    let roots = if inputs.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        inputs.to_vec()
    };
    let mut targets = BTreeMap::<PathBuf, FileTarget>::new();

    for root in roots {
        if root.is_file() {
            if !is_matlab_file(&root) {
                return Err(DiscoveryError::NotMatlabFile(root));
            }
            let matcher = ExcludeMatcher::new(
                root.parent().unwrap_or_else(|| Path::new(".")),
                exclude_patterns,
            )?;
            if !matcher.excluded(&root, false) {
                insert_target(&mut targets, root.clone(), root);
            }
            continue;
        }

        if !root.is_dir() {
            return Err(DiscoveryError::MissingPath(root));
        }

        let matcher = ExcludeMatcher::new(&root, exclude_patterns)?;
        let walker = WalkBuilder::new(&root)
            .standard_filters(true)
            .follow_links(false)
            .build();

        for entry in walker {
            let entry = entry.map_err(DiscoveryError::Walk)?;
            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() || !is_matlab_file(entry.path()) {
                continue;
            }

            let path = entry.into_path();
            if matcher.excluded(&path, false) {
                continue;
            }
            insert_target(&mut targets, path.clone(), path);
        }
    }

    Ok(targets.into_values().collect())
}

pub fn discover_diff(
    base: &str,
    exclude_patterns: &[String],
) -> Result<Vec<FileTarget>, DiscoveryError> {
    let cwd = env::current_dir().map_err(DiscoveryError::Io)?;
    discover_diff_from(&cwd, base, exclude_patterns)
}

pub fn discover_changed(exclude_patterns: &[String]) -> Result<Vec<FileTarget>, DiscoveryError> {
    let cwd = env::current_dir().map_err(DiscoveryError::Io)?;
    discover_changed_from(&cwd, exclude_patterns)
}

pub fn discover_working_tree_changes(
    cwd: &Path,
    requested_vcs: DiffVcs,
) -> Result<ChangedWorkspace, DiscoveryError> {
    match requested_vcs {
        DiffVcs::Git => discover_git_working_tree(cwd),
        DiffVcs::Fossil => discover_fossil_working_tree(cwd),
        DiffVcs::Auto => {
            if command_succeeds(cwd, "git", &["rev-parse", "--show-toplevel"]) {
                discover_git_working_tree(cwd)
            } else if command_succeeds(cwd, "fossil", &["info"]) {
                discover_fossil_working_tree(cwd)
            } else {
                Err(DiscoveryError::VcsDetection)
            }
        }
    }
}

fn discover_git_working_tree(cwd: &Path) -> Result<ChangedWorkspace, DiscoveryError> {
    let root_output = run_git(cwd, ["rev-parse", "--show-toplevel"])?;
    let root_text =
        std::str::from_utf8(&root_output).map_err(|_| DiscoveryError::NonUtf8GitOutput)?;
    let root = PathBuf::from(root_text.trim());

    let tracked = run_git(
        &root,
        [
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            "HEAD",
            "--",
        ],
    )?;
    let untracked = run_git(&root, ["ls-files", "--others", "--exclude-standard", "-z"])?;

    let mut records = parse_git_name_status(&tracked)?;
    for path in nul_paths(&untracked)? {
        if is_matlab_file(&path) {
            records.push(ChangeRecord {
                kind: ChangeKind::Added,
                old_path: None,
                new_path: Some(path),
            });
        }
    }
    let records = records
        .into_iter()
        .filter_map(normalize_matlab_record)
        .collect::<Vec<_>>();

    let old_paths: Vec<PathBuf> = records
        .iter()
        .filter_map(|record| record.old_path.clone())
        .collect();
    let old_sources = git_head_sources(&root, &old_paths)?;
    let mut old_source_by_path = old_paths
        .into_iter()
        .zip(old_sources)
        .collect::<BTreeMap<_, _>>();

    let mut files = Vec::new();
    for record in records {
        let kind = record.kind;
        let old_path = record.old_path;
        let new_path = record.new_path;

        let old_source = old_path
            .as_ref()
            .and_then(|path| old_source_by_path.remove(path))
            .flatten();
        let new_source = match new_path.as_ref() {
            Some(path) => Some(
                fs::read_to_string(root.join(path))
                    .map_err(|error| DiscoveryError::SourceIo(path.clone(), error))?,
            ),
            None => None,
        };

        files.push(ChangedFile {
            kind,
            old_path,
            new_path,
            old_source,
            new_source,
        });
    }

    sort_changed_files(&mut files);
    Ok(ChangedWorkspace {
        root,
        vcs: DetectedVcs::Git,
        files,
    })
}

#[derive(Debug)]
struct ChangeRecord {
    kind: ChangeKind,
    old_path: Option<PathBuf>,
    new_path: Option<PathBuf>,
}

fn normalize_matlab_record(mut record: ChangeRecord) -> Option<ChangeRecord> {
    let old_is_matlab = record.old_path.as_deref().is_some_and(is_matlab_file);
    let new_is_matlab = record.new_path.as_deref().is_some_and(is_matlab_file);
    if !old_is_matlab && !new_is_matlab {
        return None;
    }
    if old_is_matlab && !new_is_matlab {
        record.kind = ChangeKind::Deleted;
        record.new_path = None;
    } else if !old_is_matlab && new_is_matlab {
        record.kind = ChangeKind::Added;
        record.old_path = None;
    }
    Some(record)
}

fn parse_git_name_status(bytes: &[u8]) -> Result<Vec<ChangeRecord>, DiscoveryError> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| std::str::from_utf8(field).map_err(|_| DiscoveryError::NonUtf8GitOutput))
        .collect::<Result<Vec<_>, _>>()?;
    let mut index = 0;
    let mut records = Vec::new();

    while index < fields.len() {
        let status = fields[index];
        index += 1;
        let code = status
            .as_bytes()
            .first()
            .copied()
            .ok_or_else(|| DiscoveryError::Git("empty diff status".to_owned()))?;

        match code {
            b'R' => {
                let old = fields
                    .get(index)
                    .ok_or_else(|| DiscoveryError::Git("rename missing old path".to_owned()))?;
                let new = fields
                    .get(index + 1)
                    .ok_or_else(|| DiscoveryError::Git("rename missing new path".to_owned()))?;
                index += 2;
                records.push(ChangeRecord {
                    kind: ChangeKind::Renamed,
                    old_path: Some(PathBuf::from(old)),
                    new_path: Some(PathBuf::from(new)),
                });
            }
            b'C' => {
                let _old = fields
                    .get(index)
                    .ok_or_else(|| DiscoveryError::Git("copy missing old path".to_owned()))?;
                let new = fields
                    .get(index + 1)
                    .ok_or_else(|| DiscoveryError::Git("copy missing new path".to_owned()))?;
                index += 2;
                records.push(ChangeRecord {
                    kind: ChangeKind::Added,
                    old_path: None,
                    new_path: Some(PathBuf::from(new)),
                });
            }
            b'A' => {
                let path = fields
                    .get(index)
                    .ok_or_else(|| DiscoveryError::Git("addition missing path".to_owned()))?;
                index += 1;
                records.push(ChangeRecord {
                    kind: ChangeKind::Added,
                    old_path: None,
                    new_path: Some(PathBuf::from(path)),
                });
            }
            b'D' => {
                let path = fields
                    .get(index)
                    .ok_or_else(|| DiscoveryError::Git("deletion missing path".to_owned()))?;
                index += 1;
                records.push(ChangeRecord {
                    kind: ChangeKind::Deleted,
                    old_path: Some(PathBuf::from(path)),
                    new_path: None,
                });
            }
            b'M' | b'T' | b'U' => {
                let path = fields
                    .get(index)
                    .ok_or_else(|| DiscoveryError::Git("modification missing path".to_owned()))?;
                index += 1;
                records.push(ChangeRecord {
                    kind: ChangeKind::Modified,
                    old_path: Some(PathBuf::from(path)),
                    new_path: Some(PathBuf::from(path)),
                });
            }
            _ => {
                return Err(DiscoveryError::Git(format!(
                    "unsupported git diff status {status:?}"
                )));
            }
        }
    }

    Ok(records)
}

fn git_head_sources(root: &Path, paths: &[PathBuf]) -> Result<Vec<Option<String>>, DiscoveryError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut child = Command::new("git")
        .current_dir(root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(DiscoveryError::Io)?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| DiscoveryError::Git("failed to open git cat-file stdin".to_owned()))?;
        for path in paths {
            let path = path.to_str().ok_or(DiscoveryError::NonUtf8GitOutput)?;
            if path.contains('\n') || path.contains('\r') {
                return Err(DiscoveryError::Git(
                    "newline in path is not supported by diff-check".to_owned(),
                ));
            }
            writeln!(stdin, "HEAD:{path}").map_err(DiscoveryError::Io)?;
        }
    }

    let output = child.wait_with_output().map_err(DiscoveryError::Io)?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(DiscoveryError::Git(if message.is_empty() {
            format!("git cat-file exited with {}", output.status)
        } else {
            message
        }));
    }

    let mut cursor = 0;
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let header_end = output.stdout[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| cursor + offset)
            .ok_or_else(|| DiscoveryError::Git("truncated git cat-file header".to_owned()))?;
        let header = std::str::from_utf8(&output.stdout[cursor..header_end])
            .map_err(|_| DiscoveryError::NonUtf8GitOutput)?;
        cursor = header_end + 1;

        if header.ends_with(" missing") {
            sources.push(None);
            continue;
        }

        let size = header
            .split_whitespace()
            .last()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| {
                DiscoveryError::Git(format!("invalid git cat-file header {header:?}"))
            })?;
        if cursor + size > output.stdout.len() {
            return Err(DiscoveryError::Git(
                "truncated git cat-file content".to_owned(),
            ));
        }
        let source = String::from_utf8(output.stdout[cursor..cursor + size].to_vec())
            .map_err(|_| DiscoveryError::NonUtf8Source(path.clone()))?;
        cursor += size;
        if output.stdout.get(cursor) == Some(&b'\n') {
            cursor += 1;
        }
        sources.push(Some(source));
    }

    Ok(sources)
}

fn discover_fossil_working_tree(cwd: &Path) -> Result<ChangedWorkspace, DiscoveryError> {
    let info = run_fossil(cwd, &["info"])?;
    let info = std::str::from_utf8(&info).map_err(|_| DiscoveryError::NonUtf8FossilOutput)?;
    let root = fossil_local_root(info)?;
    let checkout = fossil_checkout_id(info)?;

    let changes = run_fossil(
        &root,
        &["changes", "--hash", "--classify", "--rel-paths"],
    )?;
    let extras = run_fossil(&root, &["extras", "--rel-paths"])?;
    let changes = std::str::from_utf8(&changes).map_err(|_| DiscoveryError::NonUtf8FossilOutput)?;
    let extras = std::str::from_utf8(&extras).map_err(|_| DiscoveryError::NonUtf8FossilOutput)?;

    let mut records = parse_fossil_changes(changes)?;
    for line in extras
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let path = PathBuf::from(line);
        if is_matlab_file(&path) {
            records.push(ChangeRecord {
                kind: ChangeKind::Added,
                old_path: None,
                new_path: Some(path),
            });
        }
    }

    let records = records
        .into_iter()
        .filter_map(normalize_matlab_record)
        .collect::<Vec<_>>();
    let old_paths = records
        .iter()
        .filter_map(|record| record.old_path.clone())
        .collect::<Vec<_>>();
    let mut old_sources = fossil_baseline_sources(&root, &checkout, &old_paths)?;

    let mut files = Vec::new();
    for record in records {
        let kind = record.kind;
        let old_path = record.old_path;
        let new_path = record.new_path;

        let old_source = old_path.as_ref().and_then(|path| old_sources.remove(path));
        let new_source = match new_path.as_ref() {
            Some(path) => Some(
                fs::read_to_string(root.join(path))
                    .map_err(|error| DiscoveryError::SourceIo(path.clone(), error))?,
            ),
            None => None,
        };

        files.push(ChangedFile {
            kind,
            old_path,
            new_path,
            old_source,
            new_source,
        });
    }

    sort_changed_files(&mut files);
    Ok(ChangedWorkspace {
        root,
        vcs: DetectedVcs::Fossil,
        files,
    })
}

fn fossil_local_root(info: &str) -> Result<PathBuf, DiscoveryError> {
    info.lines()
        .find_map(|line| line.strip_prefix("local-root:").map(str::trim))
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| DiscoveryError::Fossil("fossil info did not report local-root".to_owned()))
}

fn fossil_checkout_id(info: &str) -> Result<String, DiscoveryError> {
    info.lines()
        .find_map(|line| line.strip_prefix("checkout:").map(str::trim))
        .and_then(|value| value.split_whitespace().next())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| DiscoveryError::Fossil("fossil info did not report checkout".to_owned()))
}

fn parse_fossil_changes(changes: &str) -> Result<Vec<ChangeRecord>, DiscoveryError> {
    let mut records = Vec::new();
    for line in changes
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let mut parts = line.splitn(2, char::is_whitespace);
        let status = parts.next().unwrap_or_default();
        let remainder = parts.next().unwrap_or_default().trim();
        if remainder.is_empty() {
            return Err(DiscoveryError::Fossil(format!(
                "invalid fossil changes line {line:?}"
            )));
        }

        if let Some((old, new)) = remainder.split_once(" -> ") {
            records.push(ChangeRecord {
                kind: ChangeKind::Renamed,
                old_path: Some(PathBuf::from(old.trim())),
                new_path: Some(PathBuf::from(new.trim())),
            });
            continue;
        }

        let kind = match status {
            "ADDED" | "ADDED_BY_MERGE" | "ADDED_BY_INTEGRATE" => ChangeKind::Added,
            "DELETED" | "MISSING" => ChangeKind::Deleted,
            "RENAMED" => {
                return Err(DiscoveryError::Fossil(format!(
                    "fossil reported rename without old/new paths: {line}"
                )));
            }
            "EDITED" | "CONFLICT" | "MERGED" | "UPDATED_BY_MERGE" | "UPDATED_BY_INTEGRATE" => {
                ChangeKind::Modified
            }
            _ => {
                return Err(DiscoveryError::Fossil(format!(
                    "unsupported fossil change status {status:?}"
                )));
            }
        };
        let path = PathBuf::from(remainder);
        records.push(ChangeRecord {
            kind,
            old_path: (kind != ChangeKind::Added).then(|| path.clone()),
            new_path: (kind != ChangeKind::Deleted).then_some(path),
        });
    }
    Ok(records)
}

fn fossil_baseline_sources(
    root: &Path,
    checkout: &str,
    paths: &[PathBuf],
) -> Result<BTreeMap<PathBuf, String>, DiscoveryError> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut quoted_paths = Vec::with_capacity(paths.len());
    for path in paths {
        let text = path.to_str().ok_or(DiscoveryError::NonUtf8FossilOutput)?;
        if text
            .chars()
            .any(|character| matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(DiscoveryError::Fossil(
                "newline or tab in path is not supported by diff-check".to_owned(),
            ));
        }
        quoted_paths.push(format!("'{}'", text.replace('\'', "''")));
    }
    // `vfile` retains deleted entries and the pre-rename path, while `rid`
    // identifies the file content from the checkout baseline.
    let query = format!(
        "SELECT COALESCE(v.origname, v.pathname) AS old_path, b.size \
         FROM localdb.vfile AS v JOIN repository.blob AS b ON b.rid = v.rid \
         WHERE v.vid = (SELECT CAST(value AS INTEGER) FROM localdb.vvar WHERE name = 'checkout') \
         AND v.rid > 0 \
         AND COALESCE(v.origname, v.pathname) IN ({}) \
         ORDER BY old_path;",
        quoted_paths.join(",")
    );
    let output = run_fossil(
        root,
        &[
            "sql",
            "--readonly",
            "-batch",
            "-noheader",
            "-list",
            "-separator",
            "\t",
            &query,
        ],
    )?;
    let output = std::str::from_utf8(&output).map_err(|_| DiscoveryError::NonUtf8FossilOutput)?;

    let mut entries = Vec::new();
    for line in output.lines() {
        let (path, size) = line.split_once('\t').ok_or_else(|| {
            DiscoveryError::Fossil("unexpected output from fossil sql".to_owned())
        })?;
        let size = size
            .parse::<usize>()
            .map_err(|_| DiscoveryError::Fossil(format!("invalid baseline size for {path:?}")))?;
        entries.push((PathBuf::from(path), size));
    }

    for path in paths {
        if !entries.iter().any(|(entry, _)| entry == path) {
            return Err(DiscoveryError::Fossil(format!(
                "baseline source not found for {}",
                path.display()
            )));
        }
    }

    let mut command = Command::new("fossil");
    command.current_dir(root).arg("cat").arg("-r").arg(checkout);
    for (path, _) in &entries {
        command.arg(path);
    }
    let output = command.output().map_err(DiscoveryError::Io)?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(DiscoveryError::Fossil(if message.is_empty() {
            format!("fossil cat exited with {}", output.status)
        } else {
            message
        }));
    }

    let mut sources = BTreeMap::new();
    let mut cursor: usize = 0;
    for (path, size) in entries {
        let end = cursor
            .checked_add(size)
            .ok_or_else(|| DiscoveryError::Fossil("baseline content size overflow".to_owned()))?;
        let bytes = output
            .stdout
            .get(cursor..end)
            .ok_or_else(|| DiscoveryError::Fossil("truncated output from fossil cat".to_owned()))?;
        let source = std::str::from_utf8(bytes)
            .map_err(|_| DiscoveryError::NonUtf8Source(path.clone()))?
            .to_owned();
        sources.insert(path, source);
        cursor = end;
    }
    if cursor != output.stdout.len() {
        return Err(DiscoveryError::Fossil(
            "unexpected trailing output from fossil cat".to_owned(),
        ));
    }

    Ok(sources)
}

fn sort_changed_files(files: &mut [ChangedFile]) {
    files.sort_by(|left, right| {
        let left_path = left.new_path.as_ref().or(left.old_path.as_ref());
        let right_path = right.new_path.as_ref().or(right.old_path.as_ref());
        left_path.cmp(&right_path)
    });
}

pub fn discover_diff_from(
    cwd: &Path,
    base: &str,
    exclude_patterns: &[String],
) -> Result<Vec<FileTarget>, DiscoveryError> {
    let root_output = run_git(cwd, ["rev-parse", "--show-toplevel"])?;
    let root_text =
        std::str::from_utf8(&root_output).map_err(|_| DiscoveryError::NonUtf8GitOutput)?;
    let repo_root = PathBuf::from(root_text.trim());

    let merge_base_output = run_git(&repo_root, ["merge-base", base, "HEAD"])?;
    let merge_base =
        std::str::from_utf8(&merge_base_output).map_err(|_| DiscoveryError::NonUtf8GitOutput)?;
    let merge_base = merge_base.trim();
    if merge_base.is_empty() {
        return Err(DiscoveryError::Git(
            "git merge-base returned no commit".to_owned(),
        ));
    }

    let tracked = run_git(
        &repo_root,
        [
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=ACMR",
            "--find-renames",
            merge_base,
            "--",
        ],
    )?;
    let untracked = run_git(
        &repo_root,
        ["ls-files", "--others", "--exclude-standard", "-z"],
    )?;

    collect_git_targets(&repo_root, &tracked, &untracked, exclude_patterns)
}

pub fn discover_changed_from(
    cwd: &Path,
    exclude_patterns: &[String],
) -> Result<Vec<FileTarget>, DiscoveryError> {
    let root_output = run_git(cwd, ["rev-parse", "--show-toplevel"])?;
    let root_text =
        std::str::from_utf8(&root_output).map_err(|_| DiscoveryError::NonUtf8GitOutput)?;
    let repo_root = PathBuf::from(root_text.trim());

    let tracked = run_git(
        &repo_root,
        [
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=ACMR",
            "--find-renames",
            "HEAD",
            "--",
        ],
    )?;
    let untracked = run_git(
        &repo_root,
        ["ls-files", "--others", "--exclude-standard", "-z"],
    )?;

    collect_git_targets(&repo_root, &tracked, &untracked, exclude_patterns)
}

fn collect_git_targets(
    repo_root: &Path,
    tracked: &[u8],
    untracked: &[u8],
    exclude_patterns: &[String],
) -> Result<Vec<FileTarget>, DiscoveryError> {
    let matcher = ExcludeMatcher::new(repo_root, exclude_patterns)?;
    let mut targets = BTreeMap::<PathBuf, FileTarget>::new();

    for relative in nul_paths(tracked)?.chain(nul_paths(untracked)?) {
        if !is_matlab_file(&relative) {
            continue;
        }

        let io_path = repo_root.join(&relative);
        if matcher.excluded(&io_path, false) {
            continue;
        }
        if io_path.is_file() {
            insert_target(&mut targets, relative.clone(), io_path);
        }
    }

    Ok(targets.into_values().collect())
}

fn insert_target(
    targets: &mut BTreeMap<PathBuf, FileTarget>,
    display_path: PathBuf,
    io_path: PathBuf,
) {
    targets.entry(display_path.clone()).or_insert(FileTarget {
        io_path,
        display_path,
    });
}

fn is_matlab_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("m")
}

fn nul_paths(bytes: &[u8]) -> Result<impl Iterator<Item = PathBuf> + '_, DiscoveryError> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| {
            std::str::from_utf8(field)
                .map(PathBuf::from)
                .map_err(|_| DiscoveryError::NonUtf8GitOutput)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(fields.into_iter())
}

fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<Vec<u8>, DiscoveryError> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(DiscoveryError::Io)?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(DiscoveryError::Git(if message.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            message
        }));
    }

    Ok(output.stdout)
}

fn run_fossil(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, DiscoveryError> {
    let output = Command::new("fossil")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(DiscoveryError::Io)?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(DiscoveryError::Fossil(if message.is_empty() {
            format!("fossil exited with {}", output.status)
        } else {
            message
        }));
    }

    Ok(output.stdout)
}

fn command_succeeds(cwd: &Path, program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

struct ExcludeMatcher {
    matcher: Gitignore,
}

impl ExcludeMatcher {
    fn new(root: &Path, patterns: &[String]) -> Result<Self, DiscoveryError> {
        let mut builder = GitignoreBuilder::new(root);
        for pattern in patterns {
            builder
                .add_line(None, pattern)
                .map_err(|error| DiscoveryError::ExcludePattern {
                    pattern: pattern.clone(),
                    message: error.to_string(),
                })?;
        }
        let matcher = builder
            .build()
            .map_err(|error| DiscoveryError::ExcludePattern {
                pattern: "<combined>".to_owned(),
                message: error.to_string(),
            })?;
        Ok(Self { matcher })
    }

    fn excluded(&self, path: &Path, is_dir: bool) -> bool {
        self.matcher
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore()
    }
}

#[derive(Debug)]
pub enum DiscoveryError {
    Io(std::io::Error),
    Walk(ignore::Error),
    MissingPath(PathBuf),
    NotMatlabFile(PathBuf),
    Git(String),
    Fossil(String),
    VcsDetection,
    NonUtf8GitOutput,
    NonUtf8FossilOutput,
    NonUtf8Source(PathBuf),
    SourceIo(PathBuf, std::io::Error),
    ExcludePattern { pattern: String, message: String },
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Walk(error) => write!(formatter, "{error}"),
            Self::MissingPath(path) => write!(formatter, "{} does not exist", path.display()),
            Self::NotMatlabFile(path) => write!(formatter, "{} is not a .m file", path.display()),
            Self::Git(message) => write!(formatter, "git error: {message}"),
            Self::Fossil(message) => write!(formatter, "fossil error: {message}"),
            Self::VcsDetection => {
                write!(formatter, "could not detect a Git or Fossil working tree")
            }
            Self::NonUtf8GitOutput => write!(formatter, "git returned a non-UTF-8 path"),
            Self::NonUtf8FossilOutput => write!(formatter, "fossil returned non-UTF-8 output"),
            Self::NonUtf8Source(path) => {
                write!(formatter, "{} is not valid UTF-8", path.display())
            }
            Self::SourceIo(path, error) => write!(formatter, "{}: {error}", path.display()),
            Self::ExcludePattern { pattern, message } => {
                write!(formatter, "invalid exclude pattern {pattern:?}: {message}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "mstyle-discovery-{name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp directory");
        path
    }

    #[test]
    fn recursive_discovery_filters_extensions_and_config_excludes() {
        let root = temp_dir("recursive");
        fs::create_dir_all(root.join("src")).expect("src");
        fs::create_dir_all(root.join("vendor")).expect("vendor");
        fs::write(root.join("src/a.m"), "x = 1;\n").expect("m");
        fs::write(root.join("src/a.txt"), "x").expect("txt");
        fs::write(root.join("vendor/b.m"), "x = 2;\n").expect("vendor");

        let targets = discover_paths(std::slice::from_ref(&root), &["**/vendor/**".to_owned()])
            .expect("discover");

        assert_eq!(targets.len(), 1);
        assert!(targets[0].io_path.ends_with("src/a.m"));

        fs::remove_dir_all(root).expect("cleanup");
    }
}
