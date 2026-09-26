# Releasing mstyle

Binary releases are created from version tags matching `v*`.

Example:

```bash
git tag v0.1.0
git push origin v0.1.0
```

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

1. builds the release binary with the committed `Cargo.lock`,
2. runs `mstyle --version`,
3. runs `lint` and `check` against a clean smoke fixture,
4. uploads the staged binary and checksum as workflow artifacts,
5. downloads those artifacts into a fresh job that does not install Rust, MATLAB, or Octave,
6. verifies the checksum,
7. verifies a clean source exits with `0`,
8. verifies a known lint violation exits with `1`.

Only after all packaged-binary smoke jobs pass does a tag-triggered workflow upload the assets to the GitHub Release.

The workflow also supports `workflow_dispatch` for validating packaging without creating a release. Pull requests that change the release workflow or dependency lock files exercise the packaging jobs after this workflow version is present on the default branch. Publication is restricted to `v*` tag refs.

The normal CI workflow runs the Rust and Octave validation suites and directly executes native release binaries on Linux, Windows, macOS arm64, and macOS x86_64.
