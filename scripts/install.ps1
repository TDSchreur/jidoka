# Install the latest public Factory 2 release on Windows x86-64.
$ErrorActionPreference = 'Stop'
function Install-Factory2 {
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'x86')) {
        throw 'Only Windows x86-64 is supported by this installer.'
    }
    $base = 'https://github.com/TDSchreur/tds-factory2/releases/latest/download'
    $temp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $download = Join-Path $temp 'factory2.exe'
        $checksum = Join-Path $temp 'checksum'
        Invoke-WebRequest "$base/factory2-windows-x86_64.exe" -OutFile $download
        Invoke-WebRequest "$base/factory2-windows-x86_64.exe.sha256" -OutFile $checksum
        $expected = ((Get-Content -Raw $checksum).Trim() -split '\s+')[0]
        $actual = (Get-FileHash -Algorithm SHA256 $download).Hash
        if (-not $expected -or $actual -ne $expected) { throw 'Download checksum mismatch.' }
        $bin = Join-Path $env:LOCALAPPDATA 'Programs\factory2'
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        $destination = Join-Path $bin 'factory2.exe'
        Copy-Item -Force $download $destination
        $agent = if ($env:FACTORY2_AGENT) { $env:FACTORY2_AGENT } else { 'codex' }
        & $destination install --agent $agent
        if ($LASTEXITCODE -ne 0) { throw 'Skill installation failed.' }
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($null -eq $userPath) { $userPath = '' }
        if (($userPath -split ';') -notcontains $bin) {
            [Environment]::SetEnvironmentVariable('Path', (($userPath.TrimEnd(';') + ';' + $bin).TrimStart(';')), 'User')
        }
        if (($env:Path -split ';') -notcontains $bin) { $env:Path += ';' + $bin }
        Write-Host 'Installed factory2. Run factory2 dashboard.'
    }
    finally { Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue }
}
Install-Factory2
