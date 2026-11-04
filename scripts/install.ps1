# Install the latest public TDS Factory release on Windows x86-64.
$ErrorActionPreference = 'Stop'
function Install-Factory {
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'x86')) {
        throw 'Only Windows x86-64 is supported by this installer.'
    }
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
        $agent = if ($env:FACTORY_AGENT) { $env:FACTORY_AGENT } else { 'codex' }
        & $destination install --agent $agent
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
