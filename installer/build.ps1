# Builds the Windows installer from target\release into target\installer. Run it after
# `cargo build --release`. Needs Inno Setup 6.3 or newer (`winget install
# JRSoftware.InnoSetup`); on GitHub Actions it's installed when missing.
param(
    # The version to give the installer; Cargo.toml's when not given.
    [string]$Version
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
if (-not $Version) {
    $Version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
}

# Whether an ISCC.exe's version info says 6.3 or newer. It reads the numeric version, as
# the text one can carry a suffix. Some builds state no version at all: those are tried,
# and an old one says so when it compiles.
function Test-IsccVersion($info) {
    if (-not $info.FileVersion) { return $true }
    return $info.FileMajorPart -gt 6 -or ($info.FileMajorPart -eq 6 -and $info.FileMinorPart -ge 3)
}

# ISCC.exe of Inno Setup 6.3 or newer, if there is one: where its installer puts it (any
# major version's folder), else on the PATH.
function Find-Iscc {
    $installed = foreach ($base in ${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA "Programs")) {
        if ($base -and (Test-Path -LiteralPath $base)) {
            Get-ChildItem -LiteralPath $base -Directory -Filter "Inno Setup *" |
                ForEach-Object { Join-Path $_.FullName "ISCC.exe" } |
                Where-Object { Test-Path -LiteralPath $_ }
        }
    }
    $onPath = @(Get-Command ISCC.exe -ErrorAction SilentlyContinue | ForEach-Object Source)
    foreach ($iscc in @($installed) + $onPath) {
        $info = (Get-Item -LiteralPath $iscc).VersionInfo
        if (Test-IsccVersion $info) { return $iscc }
        Write-Host "Not using $iscc, version $($info.FileVersion): 6.3 or newer is needed"
    }
}

$iscc = Find-Iscc
if (-not $iscc -and $env:GITHUB_ACTIONS) {
    choco upgrade innosetup --yes --no-progress | Out-Host
    $iscc = Find-Iscc
}
if (-not $iscc) {
    throw "Inno Setup 6.3 or newer isn't installed. Install it with: winget install JRSoftware.InnoSetup"
}
Write-Host "Building the $Version installer with $iscc"

& $iscc "/DVersion=$Version" "/O$(Join-Path $root 'target\installer')" (Join-Path $PSScriptRoot "input-telemetry-overlay.iss")
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed (exit code $LASTEXITCODE)" }
