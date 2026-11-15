# mstyle

Fast, conservative MATLAB/Octave formatter and linter written in Rust.

`mstyle` is designed for local development, CI/CD, and coding-agent workflows. It uses Tree-sitter for MATLAB syntax awareness, applies explicit source-range edits rather than regenerating files, and avoids structural formatting when parsing is uncertain.

## Install

Download a native binary from a tagged GitHub Release, or use the checksum-verifying installer scripts. Tagged releases provide Linux x86_64, Windows x86_64, macOS x86_64, and macOS arm64 executables. The released executable is standalone for normal `check`, `fmt`, and `lint` use; Rust, MATLAB, and Octave are not required at runtime.

Linux/macOS:

```bash
curl -fsSLo install-mstyle.sh https://raw.githubusercontent.com/precsim/matlab-lint/main/scripts/install.sh
sh install-mstyle.sh
```

Windows PowerShell:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/precsim/matlab-lint/main/scripts/install.ps1 -OutFile "$env:TEMP\install-mstyle.ps1"
& "$env:TEMP\install-mstyle.ps1"
```

The installers verify the published SHA-256 checksum and do not modify `PATH`. See [docs/installing.md](docs/installing.md) for version pinning and custom install directories.

To build from source instead:

```bash
cargo build --locked --release
```

The locally built binary is written to `target/release/mstyle` (or `mstyle.exe` on Windows).

## Quick start

Format a file or directory in place:

```bash
mstyle fmt model.m
mstyle fmt src/
```

Check formatting without writing:

```bash
mstyle fmt --check src/
```

Format source from stdin and write the result to stdout:

```bash
printf 'x=1 ;' | mstyle fmt --stdin
```

`--stdin` is stream-only and cannot be combined with paths, `--check`, or `--diff`.

Run lint diagnostics:

```bash
mstyle lint src/
```

Run formatting and lint checks together:

```bash
mstyle check src/
```

If no path is supplied, `check`, `fmt`, and `lint` operate recursively from the current directory.

## CLI help

The CLI has built-in help at every level:

```bash
mstyle --help
mstyle check --help
mstyle fmt --help
mstyle lint --help
mstyle diff-check --help
```

### Top-level usage

```text
mstyle [OPTIONS] <COMMAND>
```

Commands:

| Command | Purpose |
| --- | --- |
| `check` | Check formatting and lint diagnostics without modifying files |
| `fmt` | Format MATLAB/Octave source |
| `lint` | Run lint diagnostics without modifying files |
| `diff-check` | Analyze the current MATLAB working-tree diff and evaluate diff policy |

Global options:

| Option | Meaning |
| --- | --- |
| `--config <PATH>` | Read configuration from this file instead of `./mstyle.toml` |
| `-h, --help` | Print help |
| `-V, --version` | Print version |

Because `--config` is global, it may be supplied with any subcommand.

### `mstyle check`

```text
mstyle check [OPTIONS] [PATH]...
```

Runs formatter checks and lint diagnostics together without modifying files. With no path, discovery starts from the current directory.

| Option / argument | Meaning |
| --- | --- |
| `[PATH]...` | Files or directories to check |
| `--format <text|json>` | Diagnostic output format; default: `text` |
| `--diff <BASE>` | Check only MATLAB files changed relative to `merge-base(BASE, HEAD)`, plus current local changes |
| `--changed` | Check staged, unstaged, and non-ignored untracked MATLAB files relative to `HEAD` |

`--diff`, `--changed`, and explicit paths are mutually exclusive selection modes.

Examples:

```bash
mstyle check .
mstyle check --format json src/
mstyle check --changed
mstyle check --diff origin/main
```

### `mstyle fmt`

```text
mstyle fmt [OPTIONS] [PATH]...
```

Formats files in place by default. With no path, discovery starts from the current directory.

| Option / argument | Meaning |
| --- | --- |
| `[PATH]...` | Files or directories to format |
| `--check` | Report formatting violations without modifying files |
| `--stdin` | Read source from stdin and write formatted source to stdout |
| `--diff <BASE>` | Format-check only files changed relative to `merge-base(BASE, HEAD)`; requires `--check` |

Important combinations:

- `--stdin` cannot be combined with paths, `--check`, or `--diff`.
- `--diff` requires `--check` and cannot be combined with explicit paths.
- Plain `fmt` writes changes transactionally; `fmt --check` never writes.

Examples:

```bash
mstyle fmt model.m
mstyle fmt src/
mstyle fmt --check .
mstyle fmt --check --diff origin/main
printf 'x=1 ;' | mstyle fmt --stdin
```

### `mstyle lint`

```text
mstyle lint [OPTIONS] [PATH]...
```

Runs lint diagnostics only and never modifies files. With no path, discovery starts from the current directory.

| Option / argument | Meaning |
| --- | --- |
| `[PATH]...` | Files or directories to lint |
| `--format <text|json>` | Diagnostic output format; default: `text` |

Examples:

```bash
mstyle lint .
mstyle lint src/
mstyle lint --format json src/
```

### `mstyle diff-check`

```text
mstyle diff-check [OPTIONS]
```

Analyzes the current working tree against its checkout baseline. It reads only changed `.m` files, reports raw executable and structural metrics, emits generic syntax facts, and evaluates configured diff policy without modifying files.

Git uses `HEAD` as the baseline and includes staged, unstaged, renamed, deleted, and non-ignored untracked MATLAB files. Fossil uses the current checkout baseline and includes managed changes plus unmanaged MATLAB files from `fossil extras`.

| Option | Meaning |
| --- | --- |
| `--format <text|json>` | Report format; default: `text` |
| `--vcs <git|fossil>` | Override VCS autodetection |

Examples:

```bash
mstyle diff-check
mstyle diff-check --format json
mstyle diff-check --vcs fossil
```

Comment-only, blank-only, and syntax-proven formatting-only replacements contribute zero executable churn. Formatting equivalence is conservative around MATLAB constructs where whitespace can affect semantics. Parser recovery that overlaps changed executable source raises the result to at least `REVIEW`.

## Changed-file CI

For local work, check only staged, unstaged, and untracked MATLAB files:

```bash
mstyle check --changed
```

For gradual adoption in CI, check MATLAB files changed relative to the merge-base with another ref:

```bash
mstyle check --diff origin/main
mstyle fmt --check --diff origin/main
```

`--changed` compares tracked files with `HEAD` and also includes untracked files that are not Git-ignored. `--diff` includes branch changes from the merge-base plus current local changes. Both modes remain non-mutating.

See [docs/changed-file-ci.md](docs/changed-file-ci.md) for a GitHub Actions example.

## Configuration

By default, `mstyle` reads `./mstyle.toml` when present. Use `--config PATH` to select another file.

Example:

```toml
[format]
indent_width = 2
line_endings = "lf"

[lint]
line_length = 120

[rules]
L001 = "error"
F010 = "warning"

[exclude]
paths = ["vendor/**", "generated/**"]

[diff]
vcs = "auto"
advisory = true
generated_paths = ["generated/**"]
test_paths = ["tests/**"]
public_api_inventory = "api-files.txt"
public_api_root = "src"

[diff.thresholds.general]
added_lines = { review = 101, reject = 201 }
files_touched = { review = 4, reject = 7 }

[[diff.thresholds.profile]]
name = "sensitive"
paths = ["src/numerics/**"]
functions_touched = { review = 4, reject = 11 }

[[diff.risk]]
id = "DYNAMIC_EVALUATION_CHANGE"
severity = "review"
facts = ["call_or_index_changed"]
targets = ["eval", "evalin", "feval"]
match = "context"
```

Unknown configuration keys and unknown rule IDs are rejected.

Supported rule severity values are `"error"` and `"warning"`. Formatter rules default to error severity; lint rules default to warning severity.

## Rules

| Rule | Type | Description |
| --- | --- | --- |
| F001 | format | Remove trailing whitespace |
| F002 | format | Ensure final newline |
| F003 | format | Normalize configured line endings |
| F004 | format | Replace tabs in leading indentation |
| F005 | format | Normalize block indentation |
| F006 | format | Normalize safe comma spacing |
| F007 | format | Normalize assignment spacing |
| F008 | format | Remove whitespace before semicolons |
| F009 | format | Normalize comparison/logical operator spacing |
| F010 | format | Normalize safe binary-operator spacing |
| F011 | format | Remove whitespace around range colons |
| L001 | lint | Report lines longer than the configured limit |
| L002 | lint | Report multiple statements on one line |
| PARSE | diagnostic | Report syntax recovery/errors that block structural formatting |

Formatting remains conservative around matrix/cell literals, unary operators, transpose syntax, command form, strings/character vectors, line continuations, comments, and known parser ambiguities.

## Machine-readable diagnostics

`check` and `lint` support JSON output:

```bash
mstyle check --format json src/
mstyle lint --format json src/
```

Diagnostics include stable rule IDs, severity, file, 1-based line/column spans, 0-based half-open UTF-8 byte offsets, message, and whether the issue is fixable.

## Exit codes

- `0`: no diagnostics or violations, or a `diff-check` policy finding is advisory.
- `1`: source diagnostics, lint findings, formatting violations, parse errors, or enforced `diff-check` REVIEW/REJECT-SPLIT.
- `2`: configuration, I/O, VCS integration, parser infrastructure, or internal tool failure.

`diff-check` always emits its policy decision (`ACCEPT`, `REVIEW`, or `REJECT / SPLIT`) separately from the process exit code.

## Safety model

The formatter never regenerates an entire file from an abstract syntax tree. Auto-fixes are explicit edits against the original bytes and are required to be deterministic and conflict-free.

Structural formatting is disabled when Tree-sitter reports parse recovery/errors. In that case only explicitly safe lexical cleanup may run, such as trailing-whitespace removal, EOF-newline insertion, and configured line-ending normalization.

Formatter output is reparsed before structural changes are written, and the test suite enforces idempotence:

```text
fmt(fmt(source)) == fmt(source)
```

GNU Octave is used for runtime regression tests, but it is not treated as the MATLAB parser definition.

## Development

Reference validation:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release
cargo run --locked --release -- fmt --check tests/selfcheck
cargo run --locked --release -- lint tests/selfcheck
```

The default CI also runs GNU Octave regression tests and verifies release builds on Windows and macOS.

For performance measurement:

```bash
cargo run --locked --release --example benchmark
python3 scripts/benchmark-diff-check.py
```

See [docs/benchmark.md](docs/benchmark.md) and [docs/releasing.md](docs/releasing.md) for details.
