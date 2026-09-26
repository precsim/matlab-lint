[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$InstallDir = "",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ([string]::IsNullOrWhiteSpace($Version)) {
    if (-not [string]::IsNullOrWhiteSpace($env:MSTYLE_VERSION)) {
        $Version = $env:MSTYLE_VERSION
    } else {
        $Version = "latest"
    }
}

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    if (-not [string]::IsNullOrWhiteSpace($env:MSTYLE_INSTALL_DIR)) {
        $InstallDir = $env:MSTYLE_INSTALL_DIR
    } elseif (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        $InstallDir = Join-Path $env:LOCALAPPDATA "mstyle\bin"
    } else {
        $InstallDir = Join-Path $HOME ".local\bin"
    }
}

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
if ($architecture -ne "X64") {
    throw "Windows $architecture release binaries are not currently published"
}

$asset = "mstyle-windows-x86_64.exe"

if ($Version -eq "latest") {
    $baseUrl = "https://github.com/precsim/matlab-lint/releases/latest/download"
} elseif ($Version.StartsWith("v", [System.StringComparison]::Ordinal)) {
    $baseUrl = "https://github.com/precsim/matlab-lint/releases/download/$Version"
} else {
    throw "-Version must be 'latest' or a release tag such as v0.1.0"
}

$url = "$baseUrl/$asset"
$checksumUrl = "$url.sha256"
$target = Join-Path $InstallDir "mstyle.exe"

if ($DryRun) {
    Write-Output "asset=$asset"
    Write-Output "url=$url"
    Write-Output "checksum_url=$checksumUrl"
    Write-Output "install_path=$target"
    return
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("mstyle-install-" + [System.Guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    $download = Join-Path $tempDir $asset
    $checksum = "$download.sha256"

    Invoke-WebRequest -Uri $url -OutFile $download
    Invoke-WebRequest -Uri $checksumUrl -OutFile $checksum

    $checksumLine = (Get-Content -Raw $checksum).Trim()
    $expected = ($checksumLine -split "\s+")[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 $download).Hash.ToLowerInvariant()

    if ($expected -ne $actual) {
        throw "SHA-256 mismatch for $asset"
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -Force $download $target

    Write-Output "installed mstyle to $target"

    $pathEntries = $env:PATH -split [System.IO.Path]::PathSeparator
    if ($pathEntries -notcontains $InstallDir) {
        Write-Output "add $InstallDir to PATH to run mstyle by name"
    }
}
finally {
    if (Test-Path -LiteralPath $tempDir) {
        Remove-Item -LiteralPath $tempDir -Recurse -Force
    }
}
