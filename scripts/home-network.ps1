# PowerShell 5.1+ launcher. All repositories share this user's isolated environment.
$ErrorActionPreference = 'Stop'
function Invoke-HomeNetworkNative {
    param([string] $Executable, [string[]] $NativeArguments, [ref] $ExitCode)
    # Native stderr may be ordinary output; launch failures must still produce failure.
    $ErrorActionPreference = 'Continue'
    # Native calls update this automatic variable globally, even inside a function.
    $global:LASTEXITCODE = $null
    & $Executable @NativeArguments
    if ($null -eq $global:LASTEXITCODE) { throw "Failed to start native program: $Executable" }
    $ExitCode.Value = [int] $global:LASTEXITCODE
}
$nativeExitCode = 1
[string[]] $forwarded = @($args)
$venv = Join-Path $env:LOCALAPPDATA 'HomeNetwork\venv-v1' -ErrorAction Stop
$venvPython = Join-Path $venv 'Scripts\python.exe' -ErrorAction Stop
$helper = Join-Path $PSScriptRoot 'home_network.py' -ErrorAction Stop
$bootstrap = $null
$bootstrapArgs = @()
if (Get-Command py -ErrorAction SilentlyContinue) {
    $bootstrap = Invoke-HomeNetworkNative -Executable (Get-Command py).Source -NativeArguments @('-3', '-c', 'import sys; print(sys.executable)') -ExitCode ([ref] $nativeExitCode)
    if ($nativeExitCode -ne 0) { throw 'The Python launcher could not find Python 3.' }
} elseif (Get-Command python -ErrorAction SilentlyContinue) {
    $bootstrap = (Get-Command python).Source
}
if ($forwarded.Count -gt 0 -and $forwarded[0] -eq 'setup') {
    if (-not $bootstrap) { throw 'Install Python 3.9+ before setup.' }
    Invoke-HomeNetworkNative -Executable $bootstrap -NativeArguments @('-c', 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)') -ExitCode ([ref] $nativeExitCode)
    if ($nativeExitCode -ne 0) { throw 'Python 3.9+ is required.' }
    Invoke-HomeNetworkNative -Executable $bootstrap -NativeArguments @('-m', 'venv', $venv) -ExitCode ([ref] $nativeExitCode)
    if ($nativeExitCode -ne 0) { exit $nativeExitCode }
    Invoke-HomeNetworkNative -Executable $venvPython -NativeArguments @('-m', 'pip', 'install', '-r', (Join-Path $PSScriptRoot 'home-network-requirements.txt')) -ExitCode ([ref] $nativeExitCode)
    if ($nativeExitCode -ne 0) { exit $nativeExitCode }
    Write-Output "Home-network tools installed in $venv"
    exit 0
}
if (Test-Path -LiteralPath $venvPython) {
    $interpreter = $venvPython
} elseif ($bootstrap) {
    $interpreter = $bootstrap
} else {
    throw 'Install Python 3.9+, then run this script with setup.'
}
# Base64-encoded JSON preserves every argument through PowerShell 5.1's native parser.
# The native invocation also preserves PowerShell pipelines and an interactive console.
$json = ConvertTo-Json -InputObject (@($helper) + $forwarded) -Compress -ErrorAction Stop
$encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json))
$runner = 'import base64,json,runpy,sys;sys.argv=json.loads(base64.b64decode(sys.argv[1]));runpy.run_path(sys.argv[0],run_name=''__main__'')'
$previousEncoding = [Console]::OutputEncoding
try {
    [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false) -ErrorAction Stop
    Invoke-HomeNetworkNative -Executable $interpreter -NativeArguments @('-X', 'utf8', '-c', $runner, $encoded) -ExitCode ([ref] $nativeExitCode)
    $result = $nativeExitCode
} finally {
    [Console]::OutputEncoding = $previousEncoding
}
exit $result
