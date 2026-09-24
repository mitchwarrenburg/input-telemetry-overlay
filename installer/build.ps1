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

# ISCC.exe, if Inno Setup is installed: where its installer puts it (any major version's
# folder), else on the PATH. Its version info can't say which version it is (6.7.1's
# states 0.0.0.0); one older than 6.3 fails the compile and says why.
function Find-Iscc {
    $installed = foreach ($base in ${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA "Programs")) {
        if ($base -and (Test-Path -LiteralPath $base)) {
            Get-ChildItem -LiteralPath $base -Directory -Filter "Inno Setup *" |
                ForEach-Object { Join-Path $_.FullName "ISCC.exe" } |
                Where-Object { Test-Path -LiteralPath $_ }
        }
    }
    $onPath = @(Get-Command ISCC.exe -ErrorAction SilentlyContinue | ForEach-Object Source)
    return @($installed) + $onPath | Select-Object -First 1
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
