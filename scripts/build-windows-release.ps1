$ErrorActionPreference = 'Stop'
# Cargo gives encoded and global flags precedence over target-specific flags.
if ($env:CARGO_ENCODED_RUSTFLAGS) {
    $env:CARGO_ENCODED_RUSTFLAGS += [char]31 + '-C' + [char]31 + 'target-feature=+crt-static'
}
elseif ($env:RUSTFLAGS) {
    $env:RUSTFLAGS += ' -C target-feature=+crt-static'
}
else {
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS += ' -C target-feature=+crt-static'
}
$env:JIDOKA_RELEASE = '1'
cargo build --release --locked --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
