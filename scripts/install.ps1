# Install the latest public Jidoka release on Windows x86-64.
# `jidoka.exe install` asks which agents to install skills for; $env:JIDOKA_AGENT (for example codex,claude) and
# $env:JIDOKA_CLAUDE_CONFIG_DIRS (separated by semicolons) skip the questions.
$ErrorActionPreference = 'Stop'

# Report the native code in both decimal and its Windows hexadecimal form.
function Assert-JidokaExitCode([int]$ExitCode, [string]$Action) {
    if ($ExitCode -eq 0) { return }
    $unsigned = [BitConverter]::ToUInt32([BitConverter]::GetBytes($ExitCode), 0)
    $hex = '{0:X8}' -f $unsigned
    $detail = ''
    if ($hex -eq 'C0000135') {
        $detail = ' A required DLL is missing; install the Microsoft Visual C++ x64 Redistributable or contact support.'
    }
    throw "Jidoka $Action failed with exit code $ExitCode (0x$hex).$detail"
}

# True when Windows Defender or another antivirus blocked a file (ERROR_VIRUS_INFECTED, 225 or
# 0x800700E1). Checks the codes as well as the English text, because Windows messages are localized.
function Test-JidokaAntivirusBlock([Exception]$Exception) {
    for ($e = $Exception; $e; $e = $e.InnerException) {
        if ($e.HResult -eq -2147024671 -or ($e -is [ComponentModel.Win32Exception] -and $e.NativeErrorCode -eq 225)) { return $true }
        if ($e.Message -match '(?i)contains a virus|800700E1') { return $true }
    }
    return $false
}

function Get-JidokaAntivirusAdvice([string]$Detail) {
    return @"
Windows Defender (or another antivirus) blocked the downloaded jidoka.exe: $Detail

The release is not code-signed, so antivirus heuristics sometimes flag it as a false positive. Nothing was installed. To continue:
  1. Compare the SHA256 of the downloaded file with the .sha256 file on the release page.
  2. Report the file at https://www.microsoft.com/wdsi/filesubmission as a software developer.
  3. Only if the checksum matches, restore it from Windows Security > Protection history, or ask your administrator to allow it, and run this installer again.
See https://github.com/TDSchreur/jidoka/blob/master/docs/installation.md#windows-defender-blocks-the-download
"@
}

function Install-Jidoka {
    # Preserve native exit codes even when PowerShell 7 treats them as errors.
    $PSNativeCommandUseErrorActionPreference = $false
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'x86')) {
        throw 'Only Windows x86-64 is supported by this installer.'
    }
    $base = 'https://github.com/TDSchreur/jidoka/releases/latest/download'
    $temp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $downloaded = $false
        try {
            $download = Join-Path $temp 'jidoka.exe'
            $checksum = Join-Path $temp 'checksum'
            Invoke-WebRequest "$base/jidoka-windows-x86_64.exe" -OutFile $download
            $downloaded = $true
            Invoke-WebRequest "$base/jidoka-windows-x86_64.exe.sha256" -OutFile $checksum
            $expected = ((Get-Content -Raw $checksum).Trim() -split '\s+')[0]
            $actual = (Get-FileHash -Algorithm SHA256 $download).Hash
            if (-not $expected -or $actual -ne $expected) { throw 'Download checksum mismatch.' }
            & $download --version
            Assert-JidokaExitCode $LASTEXITCODE 'startup'
        }
        catch {
            # Defender may also delete the file, so the next step fails with a different error.
            $removed = $downloaded -and -not (Test-Path -LiteralPath $download)
            if ($removed -or (Test-JidokaAntivirusBlock $_.Exception)) { throw (Get-JidokaAntivirusAdvice $_.Exception.Message) }
            throw
        }
        $bin = Join-Path $env:LOCALAPPDATA 'Programs\jidoka'
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        $destination = Join-Path $bin 'jidoka.exe'
        Copy-Item -Force $download $destination
        & $destination install
        Assert-JidokaExitCode $LASTEXITCODE 'skill installation'
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($null -eq $userPath) { $userPath = '' }
        if (($userPath -split ';') -notcontains $bin) {
            [Environment]::SetEnvironmentVariable('Path', (($userPath.TrimEnd(';') + ';' + $bin).TrimStart(';')), 'User')
            Write-Host "Added $bin to your user PATH. To undo this, edit the user Path in Windows environment variables and remove that entry."
        }
        if (($env:Path -split ';') -notcontains $bin) { $env:Path += ';' + $bin }
        Write-Host 'Installed jidoka. Run jidoka dashboard.'
    }
    finally { Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue }
}
Install-Jidoka
