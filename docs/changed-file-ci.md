# Changed-file checks

For local development, check staged, unstaged, and untracked MATLAB files without choosing a base ref:

```bash
mstyle check --changed
```

`--changed` compares tracked files with `HEAD` and includes untracked `.m` files that are not ignored by Git. Deleted files are omitted. Configured `[exclude].paths` patterns remain authoritative.

For gradual adoption in CI, fetch enough Git history to resolve the branch point and run:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0

- name: Check changed MATLAB files
  run: mstyle check --diff origin/main
```

`--diff <base>` resolves `merge-base(<base>, HEAD)` and checks the surviving tracked MATLAB paths changed relative to that branch point, including staged and unstaged work. It also includes untracked `.m` files that are not ignored by Git or `mstyle.toml`.

Tracked changed files are not dropped merely because they now match `.gitignore`. Configured `[exclude].paths` patterns remain authoritative.
