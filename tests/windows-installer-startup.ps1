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
    $env:JIDOKA_AGENT = 'codex'
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
        if (Test-Path (Join-Path $tempRoot 'Programs/jidoka/jidoka.exe')) { throw 'Installer copied a failed download' }
    }
    foreach ($blocked in @('Operation did not complete successfully because the file contains a virus or potentially unwanted software.', 'Exception 0x800700E1')) {
        function Invoke-WebRequest { param($Uri, $OutFile) throw $blocked }
        try { . (Join-Path $root 'scripts/install.ps1'); throw 'Expected failure' }
        catch {
            if ($_.Exception.Message -notmatch '(?s)Defender.*blocked.*Nothing was installed.*filesubmission') { throw "Antivirus advice absent: $_" }
        }
    }
    # Defender deletes the file after the download: the checksum step then fails with another error.
    function Invoke-WebRequest {
        param($Uri, $OutFile)
        if ($Uri -like '*.sha256') { 'abc' | Set-Content $OutFile }
    }
    try { . (Join-Path $root 'scripts/install.ps1'); throw 'Expected failure' }
    catch {
        if ($_.Exception.Message -notmatch '(?s)Defender.*blocked') { throw "Removed file not reported as blocked: $_" }
    }
    if (-not (Test-JidokaAntivirusBlock ([ComponentModel.Win32Exception]::new(225)))) { throw 'Win32 error 225 not recognized' }
    if (Test-JidokaAntivirusBlock ([InvalidOperationException]::new('Download checksum mismatch.'))) { throw 'Other errors must not match' }
    try { Assert-JidokaExitCode -1073741515 'startup'; throw 'Expected failure' }
    catch {
        if ($_.Exception.Message -notmatch '0xC0000135.*DLL') { throw "Missing DLL diagnostic absent: $_" }
    }
}
finally { Remove-Item -Recurse -Force $tempRoot }

$global:LASTEXITCODE = 0
