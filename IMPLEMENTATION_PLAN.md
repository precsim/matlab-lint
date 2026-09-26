# mstyle Implementation Plan

## 1. Goal

Build `mstyle`: a very fast, conservative MATLAB/Octave formatter and linter for:

- CI/CD enforcement,
- agentic coding workflows,
- pre-commit/editor use,
- large existing MATLAB codebases where noisy rewrites are unacceptable.

The primary design constraint is **safe, deterministic source edits**. The formatter must not attempt to pretty-print an entire MATLAB syntax tree.

## 2. Non-goals

The initial implementation will not:

- replace MATLAB Code Analyzer / `codeIssues`,
- perform deep semantic analysis,
- aggressively rewrite expressions,
- automatically wrap long lines,
- reflow comments,
- align assignments or argument lists,
- normalize quote types,
- insert semicolons,
- rewrite parentheses,
- broadly reformat matrix/cell literal contents.

MATLAB whitespace can be semantically significant, especially in matrix expressions. `mstyle` must prefer leaving code unchanged over making an uncertain edit.

## 3. Proposed implementation

### Language

Use **Rust** for the CLI and rule engine.

Reasons:

- near-zero startup overhead for agent loops and CI,
- easy distribution as a single executable,
- strong support for byte-range source edits,
- straightforward JSON output,
- good cross-platform support,
- direct bindings to Tree-sitter.

### Parsing

Use `tree-sitter` with `tree-sitter-matlab` for structural information.

Pin `tree-sitter-matlab` to an exact tested Cargo version or Git revision. Parser upgrades must be deliberate changes with the parser/golden corpus rerun, because grammar changes can alter parse structure and formatter safety assumptions.

Tree-sitter is a structural aid, not a semantic oracle for MATLAB. A successful parse, or an unchanged normalized parse tree, does not by itself prove that a rewrite preserves MATLAB semantics. Maintain explicit regression fixtures for known grammar ambiguities and constructs where valid MATLAB can be parsed differently than MATLAB evaluates it; for example, the grammar's handling of compact operator forms such as `1./a` must be guarded before structural rules are allowed to edit around that region.

Do not regenerate source from the syntax tree. Preserve the original source and apply only explicit byte-range edits.

Conceptual pipeline:

```text
files/config
    |
source bytes
    |
    +--> lexical/trivia scan
    |
    +--> tree-sitter parse
             |
             v
         rule engine
      /       |       \
 formatter  structural  lint
      \       |       /
         ordered edits
             |
     validate + apply
             |
        rewritten source
```

Core edit representation:

```rust
struct Edit {
    start_byte: usize,
    end_byte: usize,
    replacement: String,
    rule_id: RuleId,
}
```

Edits must be:

1. sorted,
2. conflict-free,
3. deterministic,
4. validated before writing.

Use half-open byte ranges, but do not treat ordinary range non-overlap as a complete conflict rule. In particular, multiple zero-length insertions at the same byte offset are order-dependent even though their ranges do not overlap.

The edit combiner must therefore reject or explicitly coalesce ambiguous combinations, including:

- multiple insertions at the same byte boundary,
- duplicate edits with different replacements,
- an insertion on a replacement/deletion boundary when ordering would affect output,
- overlapping replacement/deletion ranges.

Identical duplicate edits may be deduplicated. Any permitted coalescing or rule precedence must be explicit and covered by tests; otherwise fail safely instead of choosing an incidental iteration order.

## 4. CLI surface

Initial commands:

```bash
mstyle check .
mstyle check file.m
mstyle check --format json .
mstyle fmt .
mstyle fmt file.m
mstyle fmt --check .
mstyle lint .
mstyle lint --format json .
```

Add changed-file support once the core formatter is stable:

```bash
mstyle check --diff origin/main
mstyle fmt --check --diff origin/main
```

Optional later convenience:

```bash
mstyle check --changed
mstyle fmt --stdin
```

Command semantics:

- `fmt` applies formatter fixes and writes files transactionally.
- `fmt --check` is non-mutating and reports formatter violations only.
- `lint` is non-mutating and reports lint diagnostics only.
- `check` is the non-mutating aggregate used by CI and agents: it runs the equivalent of `fmt --check` plus `lint` over the same file set and emits one deterministically ordered diagnostic stream.
- `check --format json` and `lint --format json` use the same diagnostic schema.

Expected exit codes:

- `0`: success, with no enabled rule violations,
- `1`: source diagnostics, including style/lint violations and parse errors in input files,
- `2`: configuration, I/O, Git integration, or internal tool failure.

A parse error is a property of the source being checked, not an internal failure. Mutating `fmt` may still apply explicitly safe lexical cleanup to malformed source, but it must report the parse diagnostic and return `1`.

## 5. Configuration

Use a small `mstyle.toml`.

Example:

```toml
[format]
indent_width = 2
line_endings = "lf"

[lint]
line_length = 120

[rules]
L001 = "warning"
L002 = "error"

[exclude]
paths = [
  "external/**",
  "vendor/**",
  "build/**",
]
```

Unknown rule IDs or unsupported configuration keys must be rejected as configuration errors (exit code `2`) rather than silently ignored. This prevents misspelled or future/deferred rules from producing a false sense of enforcement.

Avoid a large configuration surface in v0.1. A formatter with too many style knobs becomes another dialect generator.

## 6. Rule model

Use three categories:

- **F**: formatter rule; deterministic auto-fix is allowed.
- **L**: lint rule; diagnostic only unless a safe fix is later proven.
- **A**: analyzer responsibility; intentionally left to MATLAB/other semantic analyzers.

### v0.1 formatter rules

Implement only low-risk rules first:

| ID | Rule | Auto-fix |
|---|---|---|
| F001 | trailing whitespace | yes |
| F002 | newline at EOF | yes |
| F003 | configured line endings | yes |
| F004 | tabs in leading indentation | yes |
| F005 | block indentation | yes |
| F006 | comma spacing where syntactically safe | yes |
| F007 | assignment spacing where syntactically safe | yes |
| F008 | whitespace immediately before semicolon | yes |

Indentation should recognize structural constructs such as:

- `function`, `classdef`,
- `methods`, `properties`, `events`, `enumeration`,
- `if`, `elseif`, `else`,
- `for`, `parfor`, `while`,
- `switch`, `case`, `otherwise`,
- `try`, `catch`,
- `spmd`,
- `arguments`,
- `end`.

Do not implement visual/alignment indentation in v0.1.

### v0.1 lint rules

Start with:

| ID | Rule |
|---|---|
| L001 | line too long |
| L002 | multiple statements per line |

Candidates for later versions:

- missing semicolon,
- file/function name mismatch,
- discouraged `global`,
- discouraged `eval`,
- wildcard import,
- naming conventions,
- duplicate local function,
- duplicate `case`,
- unreachable code after unconditional return.

## 7. Explicit preservation zones

The formatter should initially avoid internal structural/punctuation rewrites in syntax where MATLAB whitespace is risky or formatting policy is subjective.

Preserve from F004-F008 and other structural rules:

- matrix literal contents,
- cell literal contents where spacing could affect parsing,
- strings and character vectors,
- comments,
- command-form syntax,
- incomplete/error-recovery regions,
- continuation alignment beyond block indentation.

Preservation is rule-specific rather than an absolute byte-range ban. Explicitly safe lexical cleanup may touch otherwise preserved regions only where its behavior is independent of MATLAB parsing. Initially:

- F001 may remove trailing horizontal whitespace at the physical end of a line, including after a comment,
- F002 may normalize the final EOF newline,
- F003 may normalize line endings when enabled.

No structural/punctuation rule may use these lexical exceptions as permission to rewrite inside a preservation zone. Tests must assert the allowed edit domains for each rule category.

For example:

```matlab
A=[1 +1;2 -3];
```

may safely become:

```matlab
A = [1 +1;2 -3];
```

but v0.1 must not rewrite the contents of the brackets.

## 8. Rules deliberately excluded from auto-formatting

Do not automatically:

- wrap long lines,
- reflow comments,
- align consecutive assignments,
- convert single to double quotes or vice versa,
- add/remove semicolons based on style preference,
- remove optional parentheses,
- split or join expressions,
- reorder code,
- rewrite matrix/cell expression spacing,
- normalize every binary operator until a proven-safe context model exists.

These transformations create noisy diffs or can alter MATLAB semantics.

## 9. Safety model

### Parse before structural formatting

Parse each file before applying structural edits.

Use the root node's Tree-sitter error state (for example, `root_node().has_error()`) as the authoritative structural-formatting gate. Do not implement this gate by searching only for explicit `ERROR` nodes: Tree-sitter recovery can also insert `MISSING` nodes.

When the root reports an error:

- emit deterministic parse diagnostics that account for both explicit `ERROR` nodes and `MISSING` recovery nodes,
- permit only lexical rules that are provably independent of syntax, initially F001 trailing whitespace, F002 EOF newline, and optionally F003 line endings,
- do not apply indentation/operator/structural edits.

Tests must include malformed inputs that recover through explicit `ERROR` nodes and through `MISSING` nodes, and both must block F004-F008.

### Transactional formatting

Formatting flow:

1. parse original,
2. compute edits,
3. validate the complete edit set against the conflict policy,
4. apply edits in memory in one deterministic pass,
5. parse result,
6. validate the result,
7. write only if validation succeeds.

On validation failure, leave the file unchanged and return an internal safety diagnostic.

### Structural preservation

For formatter-only changes, compare normalized syntax structure before and after formatting where practical.

The comparison should ignore trivia intentionally owned by the formatter but detect meaningful syntax changes. Treat this only as a consistency check, never as proof of semantic equivalence: a grammar can parse the same bytes successfully yet assign a structure that does not match MATLAB's interpretation.

Maintain regression fixtures for known grammar ambiguities and any newly discovered mismatch. Structural rules must avoid editing ambiguous regions unless a focused guard/test establishes that the specific transformation is safe. Parser-version upgrades must rerun this corpus before merge.

This will need corpus-driven refinement because Tree-sitter error recovery and grammar ambiguities can make naive tree equality either too strict or falsely reassuring.

### Idempotence

This is a release-blocking invariant:

```text
fmt(fmt(source)) == fmt(source)
```

Every formatter fixture must have an idempotence test.

## 10. Diagnostics

Human-readable default:

```text
src/foo.m:87:121 L001 line too long (143 > 120)
src/foo.m:92:18  L002 multiple statements on one line
```

JSON mode must be stable and agent-friendly:

```json
{
  "ok": false,
  "diagnostics": [
    {
      "rule": "F007",
      "severity": "error",
      "file": "foo.m",
      "start": {"line": 32, "column": 6},
      "end": {"line": 32, "column": 7},
      "start_byte": 418,
      "end_byte": 419,
      "message": "Expected whitespace around '='",
      "fixable": true
    }
  ]
}
```

Position conventions are part of the stable JSON API:

- `line` and `column` are **1-based**,
- columns count Unicode scalar values, not UTF-8 bytes,
- `start_byte` and `end_byte` are **0-based UTF-8 byte offsets**,
- byte ranges are half-open: `[start_byte, end_byte)`.

Keep byte offsets internally and expose both byte offsets and human-readable line/column positions to users and agents. If an LSP adapter is added later, it must convert explicitly to the LSP UTF-16 position convention rather than changing this JSON contract.

## 11. Suggested Rust layout

```text
matlab-lint/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── source.rs
│   ├── parser.rs
│   ├── edit.rs
│   ├── diagnostic.rs
│   ├── formatter/
│   │   ├── mod.rs
│   │   ├── whitespace.rs
│   │   ├── indentation.rs
│   │   └── punctuation.rs
│   └── lint/
│       ├── mod.rs
│       ├── line_length.rs
│       └── statements.rs
├── tests/
│   ├── fixtures/
│   ├── selfcheck/
│   └── octave/
├── mstyle.toml
└── AGENTS.md
```

Likely dependencies:

```toml
clap
serde
serde_json
toml
tree-sitter
tree-sitter-matlab # pin exact tested version/revision
ignore
```

Keep dependencies minimal.

## 12. Testing strategy

Testing has three distinct layers.

### A. Rust unit tests

Use for:

- edit ordering and conflict rejection, including same-offset insertions and boundary conflicts,
- config parsing,
- source position calculations,
- rule behavior,
- diagnostics,
- CLI exit behavior.

These should be the fastest and most numerous tests.

### B. formatter golden tests

For each golden fixture:

```text
input.m
expected.m
```

Golden `input.m` files are intentionally allowed to violate formatter rules. They must not be included in repository-wide `fmt --check` self-checks.

Keep a separate `tests/selfcheck/` corpus containing only already-formatted MATLAB files when an end-to-end CLI self-check is useful.

Assert:

1. formatting `input.m` produces exactly `expected.m`,
2. formatting `expected.m` produces no change,
3. every formatter edit stays within the byte domains permitted for that rule category, including the narrow F001-F003 lexical exceptions,
4. output reparses successfully when input parsed successfully,
5. structural rules make no edit in a known parser-ambiguity guard region unless that transformation has a focused safety fixture.

Reparsing is a consistency check, not a semantic-equivalence proof.

Include adversarial MATLAB syntax early:

- unary `+` / `-` in matrices,
- transpose operators,
- strings vs character vectors,
- continuation lines,
- comments after continuations,
- nested blocks,
- anonymous functions,
- command syntax,
- class definitions,
- `arguments` blocks,
- cell arrays,
- indexing chains,
- malformed syntax that produces explicit `ERROR` nodes,
- malformed syntax that produces `MISSING` recovery nodes,
- known grammar ambiguities such as compact operator forms including `1./a`.

### C. GNU Octave runtime tests

Use Octave on GitHub Actions as the freely available MATLAB-compatible execution runtime.

Octave tests are **runtime smoke/regression tests**, not the parser's source-of-truth.

Use them to verify that representative source fixtures still execute after formatting.

For selected compatible fixtures:

1. run the original fixture with Octave,
2. run the formatted fixture with Octave,
3. compare explicit expected values or canonical output.

Prefer explicit `assert` checks inside test scripts over textual output comparison.

Do not weaken MATLAB syntax handling merely because a MATLAB feature is unsupported by the installed Octave version. Such syntax belongs in parser/golden tests without an Octave execution requirement.

## 13. GitHub Actions target

Primary runner:

```yaml
runs-on: ubuntu-latest
```

Environment setup:

```bash
sudo apt-get update
sudo apt-get install -y octave
rustup toolchain install stable --profile minimal
rustup default stable
```

Core CI commands once the crate is scaffolded:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

Detect the Octave executable portably on the runner:

```bash
OCTAVE_BIN="$(command -v octave-cli || command -v octave)"
"$OCTAVE_BIN" --version
```

Run Octave tests non-interactively:

```bash
"$OCTAVE_BIN" --no-gui --quiet --eval "addpath('tests/octave'); run_tests"
```

Once `mstyle` is self-hosting enough for an end-to-end corpus, run it only on the already-formatted self-check corpus:

```bash
cargo run --release -- fmt --check tests/selfcheck
cargo run --release -- lint tests/selfcheck
```

Do not run `fmt --check` across golden `input.m` fixtures; those files are deliberately unformatted test inputs.

Do not require MATLAB on the default CI path.

## 14. Changed-file / diff mode

After basic file mode is stable, add Git-aware filtering.

Requirements:

- process only `.m` files,
- allow an explicit base ref,
- resolve the comparison point as `merge-base(<base>, HEAD)`,
- include tracked files changed between that merge base and the current working tree, including staged and unstaged changes,
- include untracked `.m` files unless excluded by normal ignore/config rules,
- do not drop a tracked changed file merely because it now matches `.gitignore`,
- apply `.gitignore` to recursive discovery and untracked-file discovery,
- handle renamed and deleted paths correctly; deleted files are not parsed, while the surviving renamed path is checked,
- avoid shell-dependent Git parsing where possible.

Example:

```bash
mstyle check --diff origin/main
```

In effect, `--diff <base>` means "check the MATLAB files changed relative to the branch point with `<base>`, plus current local changes." This is the recommended initial adoption path for large legacy repositories because it avoids a repository-wide formatting commit.

## 15. Performance targets

Performance matters because agents may invoke the tool after every edit.

Initial targets on a typical GitHub Linux runner:

- CLI startup: effectively instantaneous to a human,
- single-file check: target under 25 ms for ordinary files,
- 1,000 small/medium `.m` files: target under 2 seconds excluding disk-cache cold-start variability,
- no MATLAB/Octave process startup for normal lint/format operations.

Octave is used only by the test suite, not by the linter/formatter runtime.

## 16. Implementation phases

### Phase 0 — bootstrap

Deliver:

- Rust crate,
- CLI skeleton,
- GitHub Actions workflow,
- Octave installation/runtime smoke test,
- fixture/test directory structure,
- config loading skeleton.

Acceptance:

- `cargo build --release` passes,
- Rust tests pass,
- Octave smoke test passes on `ubuntu-latest`.

### Phase 1 — source model and parser

Deliver:

- source-file abstraction,
- byte/line/column mapping,
- Tree-sitter MATLAB integration with an exact tested grammar version/revision,
- parse-error diagnostics covering `ERROR` and `MISSING` recovery,
- preservation-zone identification with rule-specific allowed edit domains,
- edit representation and deterministic conflict validation,
- parser-ambiguity regression fixtures/guards.

Acceptance:

- representative MATLAB corpus parses without crashes,
- parser errors are reported deterministically,
- both explicit `ERROR` and `MISSING` recovery block structural formatting,
- same-offset/boundary edit conflicts cannot be resolved by incidental iteration order,
- known grammar-ambiguity fixtures are guarded from unsafe structural edits,
- parser-version changes require the parser/golden corpus to pass,
- malformed input cannot trigger unsafe structural formatting.

### Phase 2 — minimal formatter

Implement:

- F001–F008,
- `fmt`,
- `fmt --check`,
- transactional writes,
- idempotence tests,
- golden fixtures.

Acceptance:

- all golden tests pass,
- every fixture is idempotent,
- matrix/cell preservation fixtures remain semantically unchanged,
- formatted Octave-compatible fixtures pass runtime assertions.

### Phase 3 — minimal linter

Implement:

- L001 line length,
- L002 multiple statements,
- severity model,
- text diagnostics,
- JSON diagnostics,
- stable exit codes.

Acceptance:

- diagnostics have stable rule IDs and source spans,
- JSON output is machine-readable and covered by snapshot/unit tests.

### Phase 4 — repository/agent workflow

Implement:

- recursive discovery,
- ignore rules,
- explicit include/exclude paths,
- `--diff <base>`,
- deterministic ordering,
- changed-file CI examples.

Acceptance:

- only intended `.m` files are visited,
- renamed/deleted paths are handled,
- output order does not vary between runs.

### Phase 5 — hardening

Deliver:

- larger real-world MATLAB corpus,
- fuzz/property tests for edit application,
- performance benchmarks,
- Windows/macOS build verification,
- binary release workflow.

Only after this phase should broader operator spacing or additional auto-fixes be considered.

## 17. Agent integration contract

The intended agent loop is:

```text
edit MATLAB
    |
mstyle fmt <changed .m files>
    |
mstyle check <changed .m files>
    |
project tests
```

Agents should consume JSON diagnostics when programmatic repair is useful.

The tool should be strict enough that repository instructions can say:

```text
Do not finish while mstyle check reports violations.
```

without requiring the agent to interpret a long prose style guide.

## 18. Definition of done for v0.1

v0.1 is complete when:

- the project builds as a standalone Rust binary,
- F001–F008 and L001–L002 are implemented,
- `check`, `fmt`, `fmt --check`, and `lint` work on files/directories,
- JSON diagnostics are stable,
- malformed syntax, including `ERROR` and `MISSING` recovery, is handled conservatively,
- formatter edits are transactional and conflict-free,
- the Tree-sitter MATLAB grammar is pinned to an exact tested version/revision,
- known parser-ambiguity fixtures are protected from unsafe structural edits,
- golden tests and idempotence tests pass,
- representative Octave-compatible fixtures execute successfully before/after formatting,
- GitHub Actions passes on `ubuntu-latest`,
- no MATLAB installation is required for build or default CI.
