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

# ISCC.exe of Inno Setup 6.3 or newer, if there is one.
function Find-Iscc {
    $candidates = @((Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source) +
        (${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA "Programs") |
            ForEach-Object { Join-Path $_ "Inno Setup 6\ISCC.exe" })
    foreach ($iscc in $candidates | Where-Object { $_ -and (Test-Path $_) }) {
        try {
            if ([version](Get-Item $iscc).VersionInfo.FileVersion -ge [version]"6.3") { return $iscc }
        } catch {
            # A version it doesn't state plainly: try the next.
        }
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

& $iscc "/DVersion=$Version" "/O$(Join-Path $root 'target\installer')" (Join-Path $PSScriptRoot "input-telemetry-overlay.iss")
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed (exit code $LASTEXITCODE)" }
