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

# ISCC.exe of Inno Setup 6.3 or newer, if there is one: on the PATH or where its installer
# puts it (any major version's folder).
function Find-Iscc {
    $onPath = @(Get-Command ISCC.exe -ErrorAction SilentlyContinue | ForEach-Object Source)
    $installed = foreach ($base in ${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA "Programs")) {
        if ($base -and (Test-Path -LiteralPath $base)) {
            Get-ChildItem -LiteralPath $base -Directory -Filter "Inno Setup *" |
                ForEach-Object { Join-Path $_.FullName "ISCC.exe" } |
                Where-Object { Test-Path -LiteralPath $_ }
        }
    }
    foreach ($iscc in $onPath + @($installed)) {
        # The numeric version: the text one can carry a suffix.
        $v = (Get-Item -LiteralPath $iscc).VersionInfo
        if ($v.FileMajorPart -gt 6 -or ($v.FileMajorPart -eq 6 -and $v.FileMinorPart -ge 3)) { return $iscc }
        Write-Host "Not using $iscc, version $($v.FileMajorPart).$($v.FileMinorPart): 6.3 or newer is needed"
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
