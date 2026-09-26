# AGENTS.md

## Scope

These instructions apply to the entire repository.

This project builds `mstyle`, a fast and conservative MATLAB/Octave formatter and linter written in Rust.

The priorities, in order, are:

1. preserve MATLAB semantics,
2. produce deterministic and idempotent formatting,
3. keep diagnostics stable for CI and agents,
4. keep startup and per-file execution fast,
5. avoid noisy source rewrites.

## Required execution environment

Use a GitHub Actions Linux runner as the reference environment:

```text
ubuntu-latest
```

The default test runtime for MATLAB-compatible execution tests is **GNU Octave**.

Do not require a MATLAB installation, MATLAB license, or MathWorks-hosted runner for the default build/test path.

## GitHub runner setup

Install Octave:

```bash
sudo apt-get update
sudo apt-get install -y octave
```

Use stable Rust:

```bash
rustup toolchain install stable --profile minimal
rustup default stable
rustc --version
cargo --version
```

Resolve the available Octave binary with:

```bash
OCTAVE_BIN="$(command -v octave-cli || command -v octave)"
test -n "$OCTAVE_BIN"
"$OCTAVE_BIN" --version
```

Prefer `octave-cli` when available to avoid GUI initialization.

## Required validation before completing a change

Once the Rust crate exists, run all of the following:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

Then run the Octave test suite:

```bash
OCTAVE_BIN="$(command -v octave-cli || command -v octave)"
"$OCTAVE_BIN" --no-gui --quiet --eval "addpath('tests/octave'); run_tests"
```

If the named test entry point does not yet exist because the repository is still in an earlier implementation phase, add the smallest appropriate smoke test as part of that phase rather than inventing an unrelated test framework.

After `mstyle` can operate on repository fixtures, also run:

```bash
cargo run --release -- fmt --check tests/fixtures
cargo run --release -- lint tests/fixtures
```

Do not claim a change is validated if any applicable command above was skipped or failed. State explicitly when a command cannot yet apply because its implementation phase has not landed.

## Octave test policy

Octave is used as a **runtime compatibility and regression test**, not as the parser definition for MATLAB.

For MATLAB/Octave-compatible fixtures:

- use Octave `assert` statements for behavior,
- execute tests non-interactively,
- ensure uncaught failures produce a non-zero process exit,
- test representative source before and/or after formatting when semantics preservation is relevant.

Prefer deterministic assertions such as:

```matlab
x = my_function(2);
assert(x == 4);
```

over comparing incidental console formatting.

Some valid MATLAB syntax may not be supported by the installed Octave version. In that case:

- keep the syntax fixture,
- test it with Rust parser/formatter golden tests,
- mark it as not requiring Octave execution,
- do not change valid MATLAB behavior just to make the Octave test pass.

Do not use Octave as justification for accepting a transformation that may change MATLAB semantics.

## Formatter safety rules

Never implement a formatter transformation by regenerating an entire file from the syntax tree.

All formatter changes must be explicit source-range edits against the original text.

Every auto-fix must satisfy these requirements:

- deterministic,
- non-overlapping with other edits,
- conservative,
- reparsed before write when structural formatting is involved,
- covered by a focused test,
- idempotent.

The invariant is:

```text
fmt(fmt(source)) == fmt(source)
```

Any formatter change that breaks idempotence is incomplete.

## MATLAB syntax preservation

Treat whitespace as potentially semantic.

Be especially conservative around:

- matrix literals,
- cell literals,
- unary `+` and `-`,
- transpose operators,
- command-form syntax,
- strings and character vectors,
- continuation lines,
- comments,
- parse-error/incomplete regions.

Do not introduce broad operator-spacing rules without syntax-aware tests proving safety.

In particular, do not normalize the inside of matrix/cell literals merely for visual consistency.

## Avoid noisy formatting

Do not add auto-fixes that:

- wrap long lines,
- reflow comments,
- vertically align assignments,
- vertically align argument lists,
- convert quote styles,
- add/remove semicolons by preference,
- remove optional parentheses,
- split/join expressions,
- reorder statements,
- reformat unrelated lines.

If such a condition is useful, prefer a diagnostic-only lint rule until a safe and stable formatter behavior is demonstrated.

## Change discipline

Keep each change narrow.

When implementing a rule:

1. add or update the rule ID,
2. add minimal positive fixtures,
3. add adversarial/negative fixtures,
4. verify unchanged code remains byte-for-byte unchanged,
5. verify expected output,
6. verify idempotence,
7. run the Rust validation commands,
8. run applicable Octave tests.

Do not combine repository-wide formatting with functional changes.

Do not modify fixtures merely to make a failing implementation test pass unless the fixture expectation is demonstrably wrong.

## Tests

Prefer this hierarchy:

### Rust unit tests

Use for:

- source ranges,
- edit application,
- config parsing,
- diagnostics,
- parser helpers,
- lint logic,
- CLI behavior.

### Golden formatter tests

Use `input.m` / `expected.m` pairs.

Each formatter fixture should assert:

1. exact expected output,
2. second formatting pass produces zero changes,
3. output reparses when the original parsed,
4. no unexpected region is modified.

### Octave runtime tests

Use only where execution adds semantic confidence.

Keep runtime fixtures small so CI remains fast.

The normal formatter/linter executable must never spawn Octave.

## Parse errors

If Tree-sitter reports syntax errors, structural formatting must stop for the affected file.

Only explicitly safe lexical cleanup may run on malformed source, initially:

- trailing whitespace,
- EOF newline,
- configured line-ending normalization if implemented safely.

Return a clear diagnostic instead of guessing indentation or expression structure.

## Diagnostics and agent compatibility

Diagnostics must have stable rule IDs.

When adding/changing JSON output:

- keep field names stable,
- include file and source span,
- include rule ID,
- include severity,
- include whether the issue is fixable,
- add tests for serialized output.

Do not emit nondeterministic diagnostic ordering. Sort by path and source location, then rule ID if necessary.

## Performance

Do not add MATLAB or Octave process startup to normal lint/format execution.

Avoid repeated parsing of the same file in one operation unless it is part of required post-format validation.

Prefer single-pass lexical analysis and reuse the Tree-sitter parse where practical.

Do not add heavy dependencies without a clear need.

## CI expectations

The primary GitHub Actions job should eventually be equivalent to:

```yaml
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - name: Install Octave
        run: |
          sudo apt-get update
          sudo apt-get install -y octave
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all-targets --all-features -- -D warnings
      - run: cargo test --all-targets --all-features
      - run: cargo build --release
      - name: Octave tests
        run: |
          OCTAVE_BIN="$(command -v octave-cli || command -v octave)"
          "$OCTAVE_BIN" --no-gui --quiet --eval "addpath('tests/octave'); run_tests"
```

When the project reaches self-check capability, add formatter/linter fixture checks rather than replacing the Rust and Octave test layers.

## Repository-wide changes

Do not run an automatic formatting pass over an existing MATLAB corpus unless the task explicitly asks for migration.

For adoption in external/legacy projects, prefer changed-file enforcement such as:

```bash
mstyle check --diff origin/main
```

until the project intentionally adopts full-repository formatting.

## Completion report

When finishing work, report concisely:

- files/rules changed,
- tests added,
- Rust validation result,
- Octave validation result,
- any known limitation or intentionally deferred rule.

Do not report success based only on compilation.
