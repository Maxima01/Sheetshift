[CmdletBinding()]
param(
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$localEnvironment = Join-Path $workspaceRoot '.tools/env.ps1'

if (Test-Path -LiteralPath $localEnvironment) {
    . $localEnvironment
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Cargo was not found. Install Rust or provide the workspace .tools/env.ps1 setup.'
}

$cargoArguments = @('test', '--locked', '-p', 'sheetshift-core', '-p', 'sheetshift-cli')
if ($Offline) {
    $cargoArguments += '--offline'
}

Push-Location $workspaceRoot
try {
    & cargo @cargoArguments
    $testExitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}

exit $testExitCode
