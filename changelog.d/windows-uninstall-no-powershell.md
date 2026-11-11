## Changed

- `jidoka uninstall` on Windows edits the user PATH directly instead of starting PowerShell, which makes antivirus false positives less likely. When the installed `jidoka.exe` is the running command, it is left in place and a command to delete it is printed.
