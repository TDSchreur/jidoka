# Install the latest public TDS Factory release on Windows x86-64.
# Choose the agents to install skills for with $env:FACTORY_AGENT (for example codex,claude)
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

function Install-Factory {
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'x86')) {
        throw 'Only Windows x86-64 is supported by this installer.'
    }
    $agents = (Get-FactoryAgents) -join ','
    $base = 'https://github.com/TDSchreur/tds-factory2/releases/latest/download'
    $temp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $download = Join-Path $temp 'factory.exe'
        $checksum = Join-Path $temp 'checksum'
        Invoke-WebRequest "$base/factory-windows-x86_64.exe" -OutFile $download
        Invoke-WebRequest "$base/factory-windows-x86_64.exe.sha256" -OutFile $checksum
        $expected = ((Get-Content -Raw $checksum).Trim() -split '\s+')[0]
        $actual = (Get-FileHash -Algorithm SHA256 $download).Hash
        if (-not $expected -or $actual -ne $expected) { throw 'Download checksum mismatch.' }
        $bin = Join-Path $env:LOCALAPPDATA 'Programs\factory'
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        $destination = Join-Path $bin 'factory.exe'
        Copy-Item -Force $download $destination
        & $destination install --agent $agents
        if ($LASTEXITCODE -ne 0) { throw 'Skill installation failed.' }
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
