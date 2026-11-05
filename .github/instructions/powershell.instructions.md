---
description: 'Write safe PowerShell scripts that run on Windows PowerShell 5.1 and PowerShell 7'
applyTo: '**/*.ps1'
---

# PowerShell scripts

- Start with `$ErrorActionPreference = 'Stop'` and `throw` with a clear message for failures. Check `$LASTEXITCODE` after calling native executables.
- Stay compatible with Windows PowerShell 5.1 as well as PowerShell 7: avoid `??`, `?:`, `&&`, and `-Parallel`.
- Use approved verb-noun names for functions, and wrap the script body in a function so a truncated `irm | iex` download does not run partially.
- Build paths with `Join-Path` and `[IO.Path]`; use `$env:LOCALAPPDATA`, not hard-coded user directories.
- Use `-LiteralPath` for paths that may contain wildcard characters, and quote paths that may contain spaces.
- Verify downloads (checksum) before copying or running them, and clean up temporary files in `finally`.
- Avoid `Invoke-Expression` on anything not fully controlled by the script. Never write secrets to output.
- Prefer `Write-Error`/`throw` for failures and `Write-Host` only for user-facing progress messages.
- Run `Invoke-ScriptAnalyzer` on changed scripts when it is available.
