# Installs the installer silently for the current user, checks what it put where, then
# uninstalls and checks it all went. For CI: it really installs.
param(
    [Parameter(Mandatory)][string]$Setup
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

$app = Join-Path $env:LOCALAPPDATA "Programs\Input Telemetry Overlay"
$exe = Join-Path $app "input-telemetry-overlay.exe"
$shortcut = Join-Path ([Environment]::GetFolderPath("Programs")) "Input Telemetry Overlay.lnk"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{A7D27628-42AE-4F92-BC37-640F8E7FBA9D}_is1"
$openCommand = "HKCU:\Software\Classes\InputTelemetryOverlay.Lap\shell\open\command"
$openWith = "HKCU:\Software\Classes\.csv\OpenWithProgids"

function Test-OpenWith {
    $key = Get-Item -LiteralPath $openWith -ErrorAction SilentlyContinue
    return [bool]($key -and $key.GetValueNames() -contains "InputTelemetryOverlay.Lap")
}

function Assert([bool]$ok, [string]$what) {
    if (-not $ok) { throw "Failed: $what" }
    Write-Host "ok: $what"
}

$run = Start-Process -FilePath $Setup -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
Assert ($run.ExitCode -eq 0) "setup exits cleanly (exit code $($run.ExitCode))"
Assert (Test-Path -LiteralPath $exe) "the program is in $app"
Assert ((Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -like "$version*") "the program is version $version"
Assert (Test-Path -LiteralPath $shortcut) "a Start menu shortcut"
$uninstall = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
Assert ([bool]$uninstall -and $uninstall.DisplayVersion -eq $version) "listed in Settings > Apps as version $version"
$command = (Get-ItemProperty -LiteralPath $openCommand -ErrorAction SilentlyContinue).'(default)'
Assert ($command -eq "`"$exe`" `"%1`"") "opening a CSV runs the program with the file ($command)"
Assert (Test-OpenWith) "offered in Open with for CSVs"

# The uninstaller hands over to a copy of itself and exits: wait for the files to go.
$run = Start-Process -FilePath (Join-Path $app "unins000.exe") -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
Assert ($run.ExitCode -eq 0) "the uninstaller starts cleanly (exit code $($run.ExitCode))"
$deadline = (Get-Date).AddSeconds(60)
while ((Test-Path -LiteralPath $exe) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
Start-Sleep -Seconds 2 # it removes the registry entries last
Assert (-not (Test-Path -LiteralPath $exe)) "uninstalling removes the program"
Assert (-not (Test-Path -LiteralPath $shortcut)) "and the shortcut"
Assert (-not (Test-Path -LiteralPath $uninstallKey)) "and the Settings > Apps entry"
Assert (-not (Test-Path -LiteralPath $openCommand)) "and the CSV handler"
Assert (-not (Test-OpenWith)) "and the Open with entry"
