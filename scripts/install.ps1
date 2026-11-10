# Install the latest public TDS Factory release on Windows x86-64.
# Choose the agents to install skills for with $env:FACTORY_AGENT (for example codex,claude)
# and the Claude config folders with $env:FACTORY_CLAUDE_CONFIG_DIRS (separated by semicolons)
# or answer the prompt.
$ErrorActionPreference = 'Stop'

# Turns a choice such as "1,3", "claude copilot" or "all" into agent names; $null when invalid.
function ConvertTo-FactoryAgents([string]$Choice) {
    $known = @{ '1' = 'codex'; '2' = 'claude'; '3' = 'copilot'; 'codex' = 'codex'; 'claude' = 'claude'; 'copilot' = 'copilot' }
    $agents = @()
    foreach ($token in ($Choice -split '[,\s]+' | Where-Object { $_ })) {
        if ($token -eq 'all') { $agents += 'codex', 'claude', 'copilot' }
        elseif ($known.ContainsKey($token)) { $agents += $known[$token] }
        else { return $null }
    }
    if (-not $agents) { return $null }
    return @($agents | Select-Object -Unique)
}

# Returns the chosen agents, asking unless FACTORY_AGENT is set.
function Get-FactoryAgents {
    if ($env:FACTORY_AGENT) {
        $agents = ConvertTo-FactoryAgents $env:FACTORY_AGENT
        if (-not $agents) { throw 'FACTORY_AGENT must list codex, claude, or copilot, separated by commas.' }
        return $agents
    }
    $noPrompt = 'No terminal to ask which agents to install for. Set $env:FACTORY_AGENT, for example codex,claude.'
    while ($true) {
        try {
            $answer = Read-Host "Install the TDS Factory skills for which agents?`n  1) codex`n  2) claude`n  3) copilot`nEnter one or more numbers or names separated by commas, or 'all'"
        }
        catch { throw $noPrompt }
        if ($null -eq $answer) { throw $noPrompt }
        $agents = ConvertTo-FactoryAgents $answer
        if ($agents) { return $agents }
        Write-Host 'Invalid choice. Try again.'
    }
}

# Report the native code in both decimal and its Windows hexadecimal form.
function Assert-FactoryExitCode([int]$ExitCode, [string]$Action) {
    if ($ExitCode -eq 0) { return }
    $unsigned = [BitConverter]::ToUInt32([BitConverter]::GetBytes($ExitCode), 0)
    $hex = '{0:X8}' -f $unsigned
    $detail = ''
    if ($hex -eq 'C0000135') {
        $detail = ' A required DLL is missing; install the Microsoft Visual C++ x64 Redistributable or contact support.'
    }
    throw "Factory $Action failed with exit code $ExitCode (0x$hex).$detail"
}

# True when Windows Defender or another antivirus blocked a file (ERROR_VIRUS_INFECTED, 225 or
# 0x800700E1). Checks the codes as well as the English text, because Windows messages are localized.
function Test-FactoryAntivirusBlock([Exception]$Exception) {
    for ($e = $Exception; $e; $e = $e.InnerException) {
        if ($e.HResult -eq -2147024671 -or ($e -is [ComponentModel.Win32Exception] -and $e.NativeErrorCode -eq 225)) { return $true }
        if ($e.Message -match '(?i)contains a virus|800700E1') { return $true }
    }
    return $false
}

function Get-FactoryAntivirusAdvice([string]$Detail) {
    return @"
Windows Defender (or another antivirus) blocked the downloaded factory.exe: $Detail

The release is not code-signed yet, so antivirus heuristics sometimes flag it as a false positive. Nothing was installed. To continue:
  1. Compare the SHA256 of the downloaded file with the .sha256 file on the release page.
  2. Report the file at https://www.microsoft.com/wdsi/filesubmission as a software developer.
  3. Only if the checksum matches, restore it from Windows Security > Protection history, or ask your administrator to allow it, and run this installer again.
See https://github.com/TDSchreur/tds-factory2#windows-defender-blocks-the-download
"@
}

function Install-Factory {
    # Preserve native exit codes even when PowerShell 7 treats them as errors.
    $PSNativeCommandUseErrorActionPreference = $false
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'x86')) {
        throw 'Only Windows x86-64 is supported by this installer.'
    }
    $agents = (Get-FactoryAgents) -join ','
    $base = 'https://github.com/TDSchreur/tds-factory2/releases/latest/download'
    $temp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $downloaded = $false
        try {
            $download = Join-Path $temp 'factory.exe'
            $checksum = Join-Path $temp 'checksum'
            Invoke-WebRequest "$base/factory-windows-x86_64.exe" -OutFile $download
            $downloaded = $true
            Invoke-WebRequest "$base/factory-windows-x86_64.exe.sha256" -OutFile $checksum
            $expected = ((Get-Content -Raw $checksum).Trim() -split '\s+')[0]
            $actual = (Get-FileHash -Algorithm SHA256 $download).Hash
            if (-not $expected -or $actual -ne $expected) { throw 'Download checksum mismatch.' }
            & $download --version
            Assert-FactoryExitCode $LASTEXITCODE 'startup'
        }
        catch {
            # Defender may also delete the file, so the next step fails with a different error.
            $removed = $downloaded -and -not (Test-Path -LiteralPath $download)
            if ($removed -or (Test-FactoryAntivirusBlock $_.Exception)) { throw (Get-FactoryAntivirusAdvice $_.Exception.Message) }
            throw
        }
        $bin = Join-Path $env:LOCALAPPDATA 'Programs\factory'
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        $destination = Join-Path $bin 'factory.exe'
        Copy-Item -Force $download $destination
        $installArgs = @('install', '--agent', $agents)
        foreach ($dir in ([string]$env:FACTORY_CLAUDE_CONFIG_DIRS -split ';')) {
            if ($dir) { $installArgs += '--config-dir', $dir }
        }
        & $destination @installArgs
        Assert-FactoryExitCode $LASTEXITCODE 'skill installation'
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($null -eq $userPath) { $userPath = '' }
        if (($userPath -split ';') -notcontains $bin) {
            [Environment]::SetEnvironmentVariable('Path', (($userPath.TrimEnd(';') + ';' + $bin).TrimStart(';')), 'User')
        }
        if (($env:Path -split ';') -notcontains $bin) { $env:Path += ';' + $bin }
        Write-Host 'Installed factory. Run factory dashboard.'
    }
    finally { Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue }
}
Install-Factory
