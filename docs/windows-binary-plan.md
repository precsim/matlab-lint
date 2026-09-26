# Windows Binary Plan

## Goal

Make the existing Windows x86_64 `mstyle.exe` release path production-ready for normal end-user use on clean Windows systems.

The repository already builds and smoke-tests:

- Rust target: `x86_64-pc-windows-msvc`
- release asset: `mstyle-windows-x86_64.exe`
- `check`, `lint`, in-place `fmt`, and `fmt --stdin`
- SHA-256 verification
- fresh-job execution without Rust, MATLAB, or Octave

This plan covers the Windows-specific hardening still worth doing before treating the binary as a polished public artifact.

## Scope

In scope:

- dependency/runtime inspection,
- clean-machine execution validation,
- Windows-native shell behavior,
- executable metadata,
- release packaging,
- signing readiness,
- SmartScreen/Defender considerations,
- path/Unicode/permission edge cases,
- release qualification.

Out of scope for the first Windows release:

- MSI installer,
- system-wide PATH mutation,
- auto-update service,
- Windows Store packaging,
- ARM64 Windows,
- bundled GUI/editor integration.

A single portable executable remains the primary distribution model.

## Phase 1 — prove the binary is self-contained enough

### Deliver

Add an explicit Windows dependency inspection step after the release build.

Record the imported DLL set for `mstyle.exe` and fail if an unexpected non-system runtime dependency appears.

Expected dependencies should be limited to Windows-provided runtime/system DLLs required by the MSVC build.

### Implementation options

Prefer a tool already available on the GitHub Windows runner, for example:

- Visual Studio `dumpbin /DEPENDENTS`, or
- PowerShell/Windows tooling that can inspect PE imports reliably.

Do not add a heavy third-party packaging dependency only for this check.

### Acceptance

- dependency list is visible in CI logs,
- no Rust toolchain files are required at runtime,
- no MATLAB/Octave installation is required,
- no project-local DLL needs to ship beside the executable,
- a newly introduced external DLL dependency fails or clearly flags CI.

## Phase 2 — Windows-native smoke suite

The current Bash-based Windows smoke test is useful, but add a PowerShell-native smoke path so quoting, redirection, exit codes, and file handling are exercised as Windows users will actually encounter them.

### Deliver

Create a small reusable PowerShell smoke script, for example:

```text
scripts/windows-smoke.ps1
```

It should test:

1. `mstyle.exe --version`,
2. `lint` clean file -> exit 0,
3. `check` clean file -> exit 0,
4. lint violation -> exit 1,
5. invalid/config/tool failure case -> exit 2 where deterministic,
6. in-place `fmt`,
7. `fmt --stdin`,
8. paths containing spaces,
9. Unicode file/path names,
10. CRLF input/output behavior,
11. read-only or unwritable-target failure handling,
12. recursive directory discovery on Windows paths.

### Acceptance

- PowerShell smoke passes on `windows-latest`,
- expected exit codes are asserted explicitly,
- exact output bytes are checked where formatting behavior matters,
- no Bash/MSYS behavior is required for the Windows qualification test.

## Phase 3 — clean Windows environment qualification

GitHub-hosted Windows runners contain developer tooling, so add a stronger approximation of a normal user machine.

### Preferred approach

Test the downloaded release artifact in a fresh Windows job that:

- does not install Rust,
- does not run Cargo,
- does not install MATLAB/Octave,
- only downloads the packaged `mstyle.exe`,
- verifies checksum,
- runs the PowerShell smoke suite.

This is already partly true in the packaged-binary job; tighten the job so no build checkout/toolchain is needed for the smoke phase.

### Optional stronger qualification

If practical later, add one of:

- Windows Sandbox test,
- minimal Windows container/VM where compatible,
- manually documented clean Windows 10/11 qualification before a release candidate.

Do not block the first release on nested virtualization if GitHub Actions cannot support it reliably.

### Acceptance

- downloaded artifact works independently of the build workspace,
- smoke job does not depend on Cargo/Rust,
- artifact works from an arbitrary temporary directory.

## Phase 4 — Windows filesystem edge cases

Add focused automated tests for Windows-specific paths and write semantics.

### Cases

- paths with spaces,
- non-ASCII/Unicode paths,
- long nested paths where runner policy allows,
- CRLF source files,
- file replacement/transactional formatting,
- existing file permissions preserved,
- read-only file failure,
- filenames with multiple dots,
- uppercase/lowercase extension behavior documented,
- working directory outside repository,
- `--config` using Windows path syntax.

### Transactional write qualification

Because Windows file replacement semantics can differ from Unix, explicitly verify that formatting:

- never leaves the temporary file on success,
- leaves the original unchanged on failed replacement,
- returns exit code 2 for I/O/tool failures,
- does not silently truncate a file.

### Acceptance

Focused integration tests pass on `windows-latest`.

## Phase 5 — executable metadata

Add Windows PE metadata where practical without complicating the Rust build.

Target metadata:

- product name: `mstyle`,
- file description: MATLAB/Octave formatter and linter,
- semantic version matching Cargo/release tag,
- project/company/copyright fields where appropriate.

Possible implementation:

- lightweight Rust build-time resource embedding such as a small `build.rs`/resource file,
- only if it remains deterministic and maintainable.

### Acceptance

Windows Explorer properties show useful version/product information and the embedded version agrees with `mstyle --version`.

This phase is useful but not release-blocking if metadata embedding introduces disproportionate complexity.

## Phase 6 — packaging

Keep the raw executable as a release asset:

```text
mstyle-windows-x86_64.exe
mstyle-windows-x86_64.exe.sha256
```

Also consider a convenience ZIP:

```text
mstyle-windows-x86_64.zip
```

containing:

```text
mstyle.exe
README.txt
LICENSE
SHA256SUMS.txt
```

The raw `.exe` should remain available for automation.

### Acceptance

- package contents are deterministic,
- checksum is published,
- ZIP extracts and runs without installation,
- executable name inside the ZIP is simply `mstyle.exe`.

## Phase 7 — code-signing readiness

Unsigned binaries can trigger Windows SmartScreen or reputation warnings. Prepare the workflow for signing without making secrets available to pull requests.

### Deliver

Define a signing stage that is:

- tag/release-only,
- after binary build/tests,
- before checksum generation/publication,
- isolated from untrusted PR execution,
- driven by repository/environment secrets.

Possible signing mechanisms:

- standard Authenticode certificate with `signtool`,
- cloud/HSM-backed signing provider if later preferred.

Do not add a real certificate or secret to the repository.

### Signing order

```text
build
  -> smoke unsigned binary
  -> sign
  -> verify signature
  -> smoke signed binary
  -> checksum/package
  -> fresh-job smoke
  -> publish
```

### Acceptance

When signing is enabled:

- `signtool verify /pa` succeeds,
- timestamping is used,
- checksum is generated from the final signed artifact,
- PR workflows have no access to signing credentials.

Signing readiness can land before a certificate is purchased.

## Phase 8 — Defender / SmartScreen qualification

Code signing and malware scanning are separate concerns.

### Deliver

For release candidates:

- run Microsoft Defender scan on the final packaged executable on the Windows runner where available,
- document that a newly signed/unsigned binary may still acquire SmartScreen reputation gradually,
- avoid binary packers/obfuscators that unnecessarily increase AV false-positive risk.

Do not weaken or disable Defender in CI.

### Acceptance

- final executable passes Defender scan in CI or a documented equivalent,
- any false positive is treated as a release blocker until investigated.

## Phase 9 — release workflow integration

Extend `.github/workflows/release.yml` so Windows-specific qualification is explicit rather than incidental.

Suggested Windows flow:

```text
cargo build --locked --release --target x86_64-pc-windows-msvc
    |
PE dependency inspection
    |
PowerShell native smoke
    |
optional Authenticode signing
    |
signature verification
    |
Defender scan
    |
checksum + ZIP/raw EXE
    |
fresh Windows artifact smoke
    |
GitHub Release
```

Keep publication gated behind all Windows qualification steps.

## Phase 10 — release-candidate checklist

Before the first public Windows release:

- [ ] locked Windows release build passes
- [ ] PE dependencies reviewed
- [ ] native PowerShell smoke passes
- [ ] clean artifact job passes
- [ ] formatting transactional-write tests pass
- [ ] spaces/Unicode/CRLF cases pass
- [ ] checksum verifies
- [ ] optional ZIP verifies
- [ ] Defender scan passes
- [ ] signing either enabled or explicitly deferred
- [ ] README documents portable Windows usage
- [ ] `mstyle --version` matches release version

## Recommended implementation order

Implement in small PRs:

1. **Windows native smoke + dependency inspection**
2. **Windows filesystem/transactional-write edge tests**
3. **Windows packaging + release documentation**
4. **PE version metadata**
5. **signing-ready release stage**
6. **enable real signing when certificate infrastructure exists**

The first three are sufficient for a strong unsigned portable Windows release. Signing should be treated as a separate operational/security step rather than blocking the formatter implementation itself.

## Definition of done

The Windows binary is release-ready when:

- `mstyle.exe` runs on a clean supported Windows x86_64 environment without Rust/MATLAB/Octave,
- no unshipped runtime dependency is required,
- CLI behavior and exit codes are verified from native PowerShell,
- formatting works correctly with Windows paths, CRLF, Unicode paths, and transactional writes,
- the exact published artifact is checksum-verified and smoke-tested,
- Defender qualification passes,
- signing is either verified or explicitly documented as deferred,
- the GitHub Release contains a directly usable portable executable.
