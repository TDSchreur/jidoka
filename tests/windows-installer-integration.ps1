param([Parameter(Mandatory = $true)][string]$Executable)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$fixture = (Resolve-Path -LiteralPath $Executable).Path
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tempRoot | Out-Null
$oldUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$oldPath = $env:Path
try {
    $env:LOCALAPPDATA = Join-Path $tempRoot 'local'
    $env:CODEX_HOME = Join-Path $tempRoot 'codex'
    $env:CLAUDE_CONFIG_DIR = Join-Path $tempRoot 'claude'
    function Invoke-WebRequest {
        param($Uri, $OutFile)
        if ($Uri -like '*.sha256') { (Get-FileHash -Algorithm SHA256 $fixture).Hash | Set-Content -LiteralPath $OutFile }
        else { Copy-Item -LiteralPath $fixture -Destination $OutFile }
    }
    function Read-Host { param($Prompt) return '2' }

    $env:FACTORY_AGENT = 'codex'
    . (Join-Path $root 'scripts/install.ps1')
    if (-not (Get-ChildItem -LiteralPath (Join-Path $env:CODEX_HOME 'skills'))) { throw 'Codex skills were not installed' }
    $bin = Join-Path $env:LOCALAPPDATA 'Programs\factory'
    if (($env:Path -split ';') -notcontains $bin) { throw 'Current shell PATH was not updated' }
    if (([Environment]::GetEnvironmentVariable('Path', 'User') -split ';') -notcontains $bin) { throw 'User PATH was not updated' }

    Remove-Item Env:FACTORY_AGENT
    . (Join-Path $root 'scripts/install.ps1')
    if (-not (Get-ChildItem -LiteralPath (Join-Path $env:CLAUDE_CONFIG_DIR 'skills'))) { throw 'Interactive selection did not install Claude skills' }
}
finally {
    [Environment]::SetEnvironmentVariable('Path', $oldUserPath, 'User')
    $env:Path = $oldPath
    Remove-Item -Recurse -Force -LiteralPath $tempRoot -ErrorAction SilentlyContinue
}
