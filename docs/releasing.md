# Releasing mstyle

Binary releases are built from a release tag that must exactly match the package version in `Cargo.toml`.

For `version = "0.1.0"`, the release tag is:

```text
v0.1.0
```

## Manual release from GitHub Actions

The preferred release path does not require creating a local Git tag:

1. open **Actions → Release binaries → Run workflow**,
2. select the `main` branch,
3. enter the matching release tag, for example `v0.1.0`,
4. run the workflow.

Before any binaries are built, the workflow verifies that:

- it was dispatched from `main`,
- the requested tag exactly matches `v<package.version>` from `Cargo.toml`,
- the tag does not already exist.

After every packaged-binary smoke test passes, the publish job creates the tag at the validated `main` commit, creates the GitHub Release, and uploads the binaries and checksums.

## Tag-triggered release

Creating and pushing the matching tag remains supported:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow validates that the pushed tag still matches `Cargo.toml` before building and publishing it.

## Release assets

The release workflow builds and publishes these explicit targets:

| Release asset | Rust target | Runner |
| --- | --- | --- |
| `mstyle-linux-x86_64` | `x86_64-unknown-linux-gnu` | `ubuntu-latest` |
| `mstyle-windows-x86_64.exe` | `x86_64-pc-windows-msvc` | `windows-latest` |
| `mstyle-macos-x86_64` | `x86_64-apple-darwin` | `macos-15-intel` |
| `mstyle-macos-arm64` | `aarch64-apple-darwin` | `macos-15` |

Each binary is accompanied by a `.sha256` checksum that can be verified from the directory containing the downloaded binary and checksum file.

## Release validation

For each target, the workflow:

1. validates the requested/pushed release tag against `Cargo.toml`,
2. builds the release binary with the committed `Cargo.lock`,
3. runs `mstyle --version`,
4. runs `lint` and `check` against a clean smoke fixture,
5. verifies in-place `fmt` output and `fmt --stdin` streaming output,
6. uploads the staged binary and checksum as workflow artifacts,
7. downloads those artifacts into a fresh job that does not install Rust, MATLAB, or Octave,
8. verifies the checksum,
9. repeats standalone formatter/linter checks from the packaged artifact,
10. verifies a clean source exits with `0`,
11. verifies a known lint violation exits with `1`.

Only after all packaged-binary smoke jobs pass does the publish job create/update the GitHub Release.

Pull requests that change the release workflow or dependency lock files exercise the validation, build, and packaged-binary smoke jobs without publishing a release.

The normal CI workflow separately runs the Rust and Octave validation suites and directly executes native release binaries on Linux, Windows, macOS arm64, and macOS x86_64.
