use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
    pub io_path: PathBuf,
    pub display_path: PathBuf,
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
    let matcher = ExcludeMatcher::new(exclude_patterns)?;
    let cwd = env::current_dir().map_err(DiscoveryError::Io)?;
    let mut targets = BTreeMap::<PathBuf, FileTarget>::new();

    for root in roots {
        if root.is_file() {
            if !is_matlab_file(&root) {
                return Err(DiscoveryError::NotMatlabFile(root));
            }
            if !matcher.excluded(match_path(&root, &cwd), false) {
                insert_target(&mut targets, root.clone(), root);
            }
            continue;
        }

        if !root.is_dir() {
            return Err(DiscoveryError::MissingPath(root));
        }

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
            if matcher.excluded(match_path(&path, &cwd), false) {
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

    let matcher = ExcludeMatcher::new(exclude_patterns)?;
    let mut targets = BTreeMap::<PathBuf, FileTarget>::new();

    for relative in nul_paths(&tracked)?.chain(nul_paths(&untracked)?) {
        if !is_matlab_file(&relative) || matcher.excluded(&relative, false) {
            continue;
        }

        let io_path = repo_root.join(&relative);
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

fn match_path<'a>(path: &'a Path, cwd: &'a Path) -> &'a Path {
    path.strip_prefix(cwd).unwrap_or(path)
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

struct ExcludeMatcher {
    matcher: Gitignore,
}

impl ExcludeMatcher {
    fn new(patterns: &[String]) -> Result<Self, DiscoveryError> {
        let mut builder = GitignoreBuilder::new(".");
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
    NonUtf8GitOutput,
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
            Self::NonUtf8GitOutput => write!(formatter, "git returned a non-UTF-8 path"),
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
