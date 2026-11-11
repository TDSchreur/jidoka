## Changed

- `jidoka check` and `jidoka update` no longer need `curl`; they contact GitHub Releases directly.
- The Windows executable now also carries a company name, copyright and an application manifest, which makes antivirus false positives less likely.
- Update downloads are https-only, size-capped, and never overwrite an existing staged file.
