# Installing mstyle

Tagged releases contain standalone binaries. Rust, MATLAB, and Octave are not required to run `mstyle`.

The installer scripts download the selected release asset, download its matching `.sha256` file, verify the SHA-256 digest, and then copy the binary into an installation directory. They do not modify `PATH`.

## Linux and macOS

Download and inspect the installer, then run it:

```bash
curl -fsSLo install-mstyle.sh https://raw.githubusercontent.com/precsim/matlab-lint/main/scripts/install.sh
sh install-mstyle.sh
```

The default installation directory is `~/.local/bin`.

Install a specific release or choose another directory:

```bash
sh install-mstyle.sh --version v0.1.0
sh install-mstyle.sh --install-dir "$HOME/bin"
```

The published Unix targets are Linux x86_64, macOS x86_64, and macOS arm64.

## Windows

From PowerShell:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/precsim/matlab-lint/main/scripts/install.ps1 -OutFile "$env:TEMP\install-mstyle.ps1"
& "$env:TEMP\install-mstyle.ps1"
```

The default installation directory is `%LOCALAPPDATA%\mstyle\bin`.

Install a specific release or choose another directory:

```powershell
& "$env:TEMP\install-mstyle.ps1" -Version v0.1.0
& "$env:TEMP\install-mstyle.ps1" -InstallDir "$HOME\bin"
```

The published Windows target is x86_64.

## CI and non-interactive use

Both installers support explicit environment overrides:

- `MSTYLE_VERSION`: `latest` or a tag such as `v0.1.0`
- `MSTYLE_INSTALL_DIR`: destination directory

Both also support a dry-run mode that resolves the platform-specific asset and prints the URLs and destination without downloading anything:

```bash
sh scripts/install.sh --dry-run
```

```powershell
.\scripts\install.ps1 -DryRun
```
