# Changed-file CI

For gradual adoption in an existing MATLAB repository, fetch enough Git history to resolve the branch point and run:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0

- name: Check changed MATLAB files
  run: cargo run --locked --release -- check --diff origin/main
```

`--diff <base>` resolves `merge-base(<base>, HEAD)` and checks the surviving tracked MATLAB paths changed relative to that branch point, including staged and unstaged work. It also includes untracked `.m` files that are not ignored by Git or `mstyle.toml`.

Tracked changed files are not dropped merely because they now match `.gitignore`. Configured `[exclude].paths` patterns remain authoritative.
