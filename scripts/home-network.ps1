# PowerShell 5.1+ launcher. All repositories share this user's isolated environment.
$ErrorActionPreference = 'Stop'
$forwarded = @($args)
$venv = Join-Path $env:LOCALAPPDATA 'HomeNetwork\venv-v1'
$venvPython = Join-Path $venv 'Scripts\python.exe'
$helper = Join-Path $PSScriptRoot 'home_network.py'
$bootstrap = $null
$bootstrapArgs = @()
if (Get-Command py -ErrorAction SilentlyContinue) {
    $bootstrap = & (Get-Command py).Source -3 -c 'import sys; print(sys.executable)'
    if ($LASTEXITCODE -ne 0) { throw 'The Python launcher could not find Python 3.' }
} elseif (Get-Command python -ErrorAction SilentlyContinue) {
    $bootstrap = (Get-Command python).Source
}
if ($forwarded.Count -gt 0 -and $forwarded[0] -eq 'setup') {
    if (-not $bootstrap) { throw 'Install Python 3.9+ before setup.' }
    & $bootstrap -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)'
    if ($LASTEXITCODE -ne 0) { throw 'Python 3.9+ is required.' }
    & $bootstrap -m venv $venv
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $venvPython -m pip install -r (Join-Path $PSScriptRoot 'home-network-requirements.txt')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
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
$json = ConvertTo-Json -InputObject (@($helper) + $forwarded) -Compress
$encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json))
$runner = 'import base64,json,runpy,sys;sys.argv=json.loads(base64.b64decode(sys.argv[1]));runpy.run_path(sys.argv[0],run_name=''__main__'')'
$previousEncoding = [Console]::OutputEncoding
try {
    [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
    & $interpreter -X utf8 -c $runner $encoded
    $result = $LASTEXITCODE
} finally {
    [Console]::OutputEncoding = $previousEncoding
}
exit $result
