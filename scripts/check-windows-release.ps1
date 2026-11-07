param([Parameter(Mandatory = $true)][string]$Executable)
$ErrorActionPreference = 'Stop'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $vs) { throw 'Visual C++ build tools were not found' }
$dumpbin = Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe') |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $dumpbin) { throw 'dumpbin was not found' }
$imports = & $dumpbin.FullName /dependents $Executable
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect executable imports' }
$imports | Out-Host
if ($imports -match '(?i)\b(vcruntime[\w-]*|msvcp[\w-]*|api-ms-win-crt-[\w-]+)\.dll\b') {
    throw 'Executable imports a dynamic Visual C++ runtime'
}
& $Executable --version
if ($LASTEXITCODE -ne 0) { throw "Release executable failed with exit code $LASTEXITCODE" }
