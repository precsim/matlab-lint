# Diff checking

`mstyle diff-check` analyzes the MATLAB portion of the current working-tree diff without modifying files:

```bash
mstyle diff-check
mstyle diff-check --format json
mstyle diff-check --vcs git
mstyle diff-check --vcs fossil
```

The command is repository-independent. Git and Fossil acquisition both produce the same old/new changed-file representation before MATLAB analysis begins. Git compares the working tree with `HEAD`; Fossil compares with the current checkout baseline. Non-MATLAB paths are ignored.

## Metrics

The report keeps measurements in their original units:

- executable lines added/deleted;
- MATLAB files touched;
- functions touched/added;
- positive control-flow complexity increase;
- `%%` sections touched/added.

Configured test paths are reported separately from implementation metrics. Configured generated paths are excluded from analysis. Blank/comment-only lines do not contribute executable churn. A replacement contributes zero churn only when clean syntax trees prove equal structure and non-comment token content; ambiguous or whitespace-sensitive MATLAB regions are treated conservatively.

## Policy

Thresholds use inclusive boundaries:

```text
value < review              ACCEPT
review <= value < reject    REVIEW
value >= reject             REJECT / SPLIT
```

General thresholds evaluate all implementation files. Every matching path profile is evaluated independently, including overlapping profiles and both sides of a rename. The final decision is the highest severity from thresholds, profiles, parse confidence, and semantic-risk rules.

Example:

```toml
[diff]
vcs = "auto"
advisory = true
generated_paths = ["generated/**"]
test_paths = ["tests/**"]
public_api_inventory = "api-files.txt"
public_api_root = "src"

[diff.thresholds.general]
added_lines       = { review = 101, reject = 201 }
deleted_lines     = { review = 101, reject = 201 }
files_touched     = { review = 4,   reject = 7 }
functions_touched = { review = 6,   reject = 11 }
functions_added   = { review = 3,   reject = 6 }
control_flow      = { review = 5,   reject = 11 }
sections_touched  = { review = 4,   reject = 8 }
sections_added    = { review = 2,   reject = 4 }

[[diff.thresholds.profile]]
name = "sensitive"
paths = ["src/numerics/**"]
added_lines = { review = 51, reject = 201 }
functions_touched = { review = 4, reject = 11 }

[[diff.risk]]
id = "PUBLIC_API_SIGNATURE_CHANGE"
severity = "review"
fact = "public_api_signature_changed"

[[diff.risk]]
id = "DYNAMIC_EVALUATION_CHANGE"
severity = "review"
facts = ["call_or_index_changed"]
targets = ["eval", "evalin", "feval"]
match = "context"
```

All diff-related paths are interpreted from the detected repository root. Public API inventory entries are relative to `public_api_root`; blank lines and lines beginning with `#` are ignored.

## Semantic facts

Facts use stable snake-case kinds:

```text
function_added
function_deleted
function_signature_changed
public_api_signature_changed
persistent_changed
global_changed
assignment_changed
numeric_expression_changed
call_or_index_changed
control_flow_changed
section_added
section_touched
```

Each fact includes side/path/range, enclosing function when known, identifiers, call-or-index targets, field paths, operators, and literals. `A(1)` deliberately remains `call_or_index_changed`; the analyzer does not claim call-versus-index semantics.

For `match = "any"`, a configured fact kind is sufficient after optional path filtering. For `match = "context"`, every non-empty selector category must match the same fact occurrence. Supported selectors are `identifiers`, `targets`, `field_paths`, `functions`, `operators`, and `literals`.

## Parse confidence and exit codes

Parser recovery overlapping changed executable source withholds unsafe structural facts and raises policy to at least `REVIEW`. Parse recovery wholly outside changed executable ranges does not change the diff decision.

The process contract is:

```text
0  successful command with no enforced finding
1  source/parse finding or enforced REVIEW / REJECT-SPLIT
2  configuration, I/O, VCS, parser infrastructure, or internal failure
```

Advisory mode suppresses only the exit status caused by policy classification. The policy decision is still reported. Source diagnostics and tool failures keep their normal nonzero status.

## Performance

CI builds the release binary and runs `scripts/benchmark-diff-check.py` against representative synthetic Git and Fossil working trees. The benchmark reports a `<500 ms` target and fails if an ordinary run reaches the required `1 s` ceiling.
