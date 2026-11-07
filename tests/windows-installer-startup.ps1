# Exercise the installer with a local executable on Linux; Windows validation is separate.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tempRoot | Out-Null
try {
    $stub = Join-Path $tempRoot 'stub'
    [IO.File]::WriteAllText($stub, "#!/bin/sh`nexit 37`n")
    & chmod +x $stub
    $env:PROCESSOR_ARCHITECTURE = 'AMD64'
    $env:LOCALAPPDATA = $tempRoot
    $env:FACTORY_AGENT = 'codex'
    $script:badChecksum = $false
    function Invoke-WebRequest {
        param($Uri, $OutFile)
        if ($Uri -like '*.sha256') {
            if ($script:badChecksum) { 'wrong' | Set-Content $OutFile }
            else { (Get-FileHash -Algorithm SHA256 $stub).Hash | Set-Content $OutFile }
        }
        else { Copy-Item $stub $OutFile }
    }
    foreach ($case in @('startup', 'checksum')) {
        $script:badChecksum = $case -eq 'checksum'
        try { . (Join-Path $root 'scripts/install.ps1'); throw 'Expected failure' }
        catch {
            $expected = if ($script:badChecksum) { 'checksum mismatch' } else { 'startup.*37.*0x00000025' }
            if ($_.Exception.Message -notmatch $expected) { throw "Unexpected $case failure: $_" }
        }
        if (Test-Path (Join-Path $tempRoot 'Programs/factory/factory.exe')) { throw 'Installer copied a failed download' }
    }
    try { Assert-FactoryExitCode -1073741515 'startup'; throw 'Expected failure' }
    catch {
        if ($_.Exception.Message -notmatch '0xC0000135.*DLL') { throw "Missing DLL diagnostic absent: $_" }
    }
}
finally { Remove-Item -Recurse -Force $tempRoot }
