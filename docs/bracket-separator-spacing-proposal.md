# Proposal: configurable separator spacing in matrix and cell literals

## Status

Proposal.

## Summary

Add an opt-in formatter style that normalizes spacing around explicit comma and semicolon separators inside MATLAB matrix and cell literals without enabling general formatting of bracket contents.

The intended style is:

```matlab
[a, b; c]
{a, b; c}
```

Existing comma formatting outside matrix/cell literals remains unchanged:

```matlab
f(a, b)
A(i, j)
```

MATLAB parentheses are not matrix constructors, so semicolon handling in `(...)` is not part of this proposal.

## Motivation

`mstyle` currently preserves matrix and cell contents because whitespace can be semantic in MATLAB. For example:

```matlab
[a b]
[a -b]
[a +b]
```

A broad rule that rewrites whitespace inside `[]` or `{}` would therefore be unsafe.

Commas and semicolons are different: they are explicit element/row separators. Formatting whitespace immediately adjacent to those separator tokens can be narrowly defined without interpreting ordinary whitespace as a delimiter.

This proposal adds that narrow style while retaining the existing conservative preservation model.

## Configuration

Add a format option with an explicit preserve default:

```toml
[format]
bracket_separator_spacing = "preserve"
```

Supported values:

- `"preserve"` — default; retain the current behavior and do not format separators inside matrix/cell literals.
- `"space"` — normalize explicit comma/semicolon separators inside matrix/cell literals.

Example opt-in configuration:

```toml
[format]
indent_width = 2
line_endings = "lf"
bracket_separator_spacing = "space"
```

The default must remain `"preserve"` to avoid introducing repository-wide source rewrites for existing users.

## Formatting behavior

When `bracket_separator_spacing = "space"`, normalize an explicit separator only when it is syntactically inside a matrix or cell literal.

For an inline separator:

1. remove horizontal whitespace immediately before `,` or `;`,
2. ensure exactly one ASCII space after the separator,
3. do not cross a line boundary,
4. do not rewrite unrelated whitespace.

Examples:

```matlab
[a,b;c]          -> [a, b; c]
{a,b;c}          -> {a, b; c}
[a ,b ;c]        -> [a, b; c]
{a,  b;   c}     -> {a, b; c}
[a,b;{c,d}]      -> [a, b; {c, d}]
```

The rule must not normalize ordinary element-separating whitespace:

```matlab
[a b]            -> unchanged
[a  b]           -> unchanged
[a -b]           -> unchanged
[a +b]           -> unchanged
[a' b]           -> unchanged
```

It must also preserve whitespace inside strings/character vectors:

```matlab
["a,b", "c;d"]
{'a,b'; 'c;d'}
```

Only separator tokens belonging to the MATLAB syntax tree are candidates; text that merely contains `,` or `;` is not.

## Line boundaries, comments, and continuations

The formatter must not join lines or insert spaces across line boundaries.

Examples that remain structurally multiline:

```matlab
A = [a,
     b];

C = {a;
     b};
```

If the separator is directly followed by a comment or line continuation, preserve the existing boundary rather than forcing an inline space rewrite unless a dedicated safe case is proven by tests.

Examples to preserve conservatively:

```matlab
A = [a, ... 
     b];

A = [a; % next row
     b];
```

No rule in this proposal may move comments, reflow comments, or alter continuation structure.

## Rule identity

Add a dedicated formatter rule:

```text
F012  Normalize configured matrix/cell separator spacing
```

A separate rule is preferable to reusing F006/F008 because:

- the behavior is opt-in,
- it applies in preservation zones that existing punctuation rules intentionally avoid,
- users and CI should be able to distinguish this style from general comma/semicolon formatting,
- tests can target its safety boundary independently.

When the option is `"preserve"`, F012 produces no edits or diagnostics.

## Implementation approach

Do not relax the existing matrix/cell preservation zones globally.

Instead, add a dedicated collector for F012 that:

1. walks the parsed Tree-sitter syntax tree,
2. identifies comma and semicolon tokens whose relevant ancestor is a matrix or cell literal,
3. emits only local source-range edits adjacent to the separator token,
4. rejects edits that cross newlines, comments, continuation regions, parse-error regions, or protected lexical content,
5. leaves all other matrix/cell whitespace untouched.

This keeps the invariant that general structural rules still cannot enter matrix/cell preservation zones.

The exact Tree-sitter node kinds should be confirmed against the pinned `tree-sitter-matlab` grammar during implementation rather than guessed in the proposal.

## Safety requirements

The implementation must satisfy the repository formatter requirements:

- explicit source-range edits only,
- deterministic output,
- non-overlapping edits,
- clean reparse before write for structural formatting,
- exact idempotence,
- no formatting when the source has structural parse errors,
- no semantic normalization of ordinary whitespace inside matrix/cell literals.

In particular:

```text
fmt(fmt(source)) == fmt(source)
```

must hold with both configuration values.

## Tests

### Positive formatter tests

Add golden/unit coverage for at least:

```matlab
[a,b;c]
{a,b;c}
[a ,b ;c]
{a,  b;   c}
[a,b;{c,d}]
{[a,b];[c,d]}
```

with `bracket_separator_spacing = "space"`.

### Preserve/default tests

Verify that the default configuration leaves existing matrix/cell fixtures byte-for-byte unchanged.

### Adversarial tests

Cover:

```matlab
[a b]
[a  b]
[a -b]
[a +b]
[a' b]
[1:3,4:6]
["a,b", "c;d"]
{'a,b'; 'c;d'}
[a, ...
 b]
[a; % comment
 b]
```

Also cover nested matrices/cells and separators adjacent to closing delimiters or multiline constructs.

### Idempotence and reparse

For every positive fixture:

1. format once and compare exact expected output,
2. format the result again and require zero edits,
3. require the formatted result to parse cleanly whenever the original parsed cleanly.

### Runtime regression

Add a small GNU Octave regression showing that representative matrices/cells have the same values before and after formatting.

For example, compare numeric matrices and nested cell contents rather than console output.

## CLI and diagnostics

No new CLI flag is required. The style is configured through `mstyle.toml` or the existing `--config` mechanism.

`fmt` applies the configured style.

`fmt --check` and `check` report F012 when the source differs from the configured style.

JSON diagnostics use the existing stable F012 rule ID and source-span conventions.

## Documentation

If implemented:

- add the configuration option to README configuration documentation,
- add F012 to the rules table,
- include a matrix/cell example showing `"preserve"` versus `"space"`,
- state clearly that ordinary whitespace inside matrix/cell literals is never normalized by this option.

## Non-goals

This proposal does not:

- normalize all whitespace inside matrices or cells,
- align matrix columns,
- enforce spaces between elements that are separated only by whitespace,
- rewrite unary `+` or `-` cases,
- wrap or unwrap matrix rows,
- alter comments or continuations,
- add semicolon formatting to parenthesized expressions,
- change quote style,
- generalize preservation-zone access for existing formatter rules.

## Acceptance criteria

The proposal is ready to implement when the chosen option name and F012 behavior are accepted.

Implementation is complete only when:

1. `"preserve"` exactly retains current behavior,
2. `"space"` produces `[a, b; c]` / `{a, b; c}` for explicit separators,
3. ordinary matrix/cell whitespace remains byte-for-byte unchanged,
4. all focused safety, golden, idempotence, and Octave tests pass,
5. all repository validation commands required by `AGENTS.md` pass.
