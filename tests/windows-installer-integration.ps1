param([Parameter(Mandatory = $true)][string]$Executable)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$fixture = (Resolve-Path -LiteralPath $Executable).Path
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tempRoot | Out-Null
$oldUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$oldPath = $env:Path
$savedEnv = @{}
foreach ($name in 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'COPILOT_HOME', 'CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'JIDOKA_AGENT', 'JIDOKA_CLAUDE_CONFIG_DIRS') {
    $savedEnv[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    # Everything the installer and `jidoka install` write (skills, install record, policy) stays in $tempRoot.
    $env:USERPROFILE = Join-Path $tempRoot 'home'
    $env:APPDATA = Join-Path $tempRoot 'roaming'
    $env:LOCALAPPDATA = Join-Path $tempRoot 'local'
    $env:COPILOT_HOME = Join-Path $tempRoot 'copilot'
    $env:CODEX_HOME = Join-Path $tempRoot 'codex'
    $env:CLAUDE_CONFIG_DIR = Join-Path $tempRoot 'claude'
    Remove-Item Env:XDG_CONFIG_HOME, Env:XDG_DATA_HOME, Env:JIDOKA_CLAUDE_CONFIG_DIRS -ErrorAction SilentlyContinue
    function Invoke-WebRequest {
        param($Uri, $OutFile)
        if ($Uri -like '*.sha256') { (Get-FileHash -Algorithm SHA256 $fixture).Hash | Set-Content -LiteralPath $OutFile }
        else { Copy-Item -LiteralPath $fixture -Destination $OutFile }
    }
    function Read-Host { throw 'The installer must not prompt itself' }

    $env:JIDOKA_AGENT = 'codex'
    $output = . (Join-Path $root 'scripts/install.ps1') 6>&1
    if (-not ($output | Where-Object { $_.ToString() -like '*Added * to your user PATH*remove that entry*' })) { throw 'PATH change and undo instructions were not reported' }
    if (-not (Get-ChildItem -LiteralPath (Join-Path $env:CODEX_HOME 'skills'))) { throw 'Codex skills were not installed' }
    $bin = Join-Path $env:LOCALAPPDATA 'Programs\jidoka'
    if (($env:Path -split ';') -notcontains $bin) { throw 'Current shell PATH was not updated' }
    if (([Environment]::GetEnvironmentVariable('Path', 'User') -split ';') -notcontains $bin) { throw 'User PATH was not updated' }

    $env:JIDOKA_AGENT = 'claude'
    $output = . (Join-Path $root 'scripts/install.ps1') 6>&1
    if ($output | Where-Object { $_.ToString() -like '*Added * to your user PATH*' }) { throw 'Installer reported an unchanged user PATH as added' }
    if (-not (Get-ChildItem -LiteralPath (Join-Path $env:CLAUDE_CONFIG_DIR 'skills'))) { throw 'JIDOKA_AGENT=claude did not install Claude skills' }

    $first = Join-Path $tempRoot 'claude work'
    $second = Join-Path $tempRoot 'claude-other'
    $default = Join-Path $tempRoot 'claude-default'
    $env:CLAUDE_CONFIG_DIR = $default
    $env:JIDOKA_AGENT = 'claude'
    $env:JIDOKA_CLAUDE_CONFIG_DIRS = "$first;;$second;"
    . (Join-Path $root 'scripts/install.ps1')
    foreach ($dir in @($first, $second)) {
        if (-not (Get-ChildItem -LiteralPath (Join-Path $dir 'skills'))) { throw "Skills were not installed in $dir" }
    }
    if (Test-Path -LiteralPath $default) { throw 'JIDOKA_CLAUDE_CONFIG_DIRS must replace the default Claude folder' }
}
finally {
    [Environment]::SetEnvironmentVariable('Path', $oldUserPath, 'User')
    $env:Path = $oldPath
    foreach ($name in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnv[$name], 'Process') }
    Remove-Item -Recurse -Force -LiteralPath $tempRoot -ErrorAction SilentlyContinue
}
