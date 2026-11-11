//! The format of the dashboard's tokens, shared by the server that creates them and the CLI that reads them back.

use std::{fs, io::Read, path::Path};

/// Random bytes in a token.
const BYTES: usize = 32;
/// Characters in a token: each byte as two lowercase hex digits.
const LENGTH: usize = BYTES * 2;
/// A token file may end in a line break (`\n` or `\r\n`); nothing more is read than that allows.
const MAX_FILE_BYTES: u64 = LENGTH as u64 + 2;

/// A new token: 32 bytes from the operating system's CSPRNG, as 64 lowercase hex characters. Fails when the operating
/// system gives no random bytes.
pub(crate) fn random() -> Result<String, String> {
    let mut bytes = [0u8; BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        format!(
            "cannot start the dashboard: the operating system gave no random bytes for its tokens: {error}"
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Whether `token` is exactly what `random` produces: 64 lowercase hex characters.
fn is_valid(token: &str) -> bool {
    token.len() == LENGTH
        && token
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Reads the token a server wrote to `path`, or `None` when the file is missing, is not a regular file, is longer than
/// a token with a line break, or holds anything other than a valid token. It never reads more than a few bytes and
/// never blocks on a pipe that is already there (see the comment below for the swap race), so a hostile or mistaken path cannot hang or bloat the CLI.
pub(crate) fn read_file(path: &Path) -> Option<String> {
    // Check the type before opening: opening a named pipe for reading waits for a writer, and `metadata` never waits.
    // The handle is checked again after opening, which catches a swap to another file type but not a swap to a pipe
    // between the two calls: that `open` would still wait for a writer, because it is not opened non-blocking.
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    // One byte more than allowed, to tell a file that is exactly long enough from one that is too long.
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    let token = std::str::from_utf8(&bytes).ok()?.trim();
    is_valid(token).then(|| token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::fs;

    #[test]
    fn random_tokens_are_valid_and_differ() {
        let (a, b) = (random().unwrap(), random().unwrap());
        assert!(is_valid(&a) && is_valid(&b));
        assert_eq!((a.len(), b.len()), (LENGTH, LENGTH));
        assert_ne!(a, b);
    }

    #[test]
    fn a_token_file_is_read_with_or_without_a_line_break() {
        let dir = TestDir::new();
        let token = random().unwrap();
        for (name, content) in [
            ("bare", token.clone()),
            ("lf", format!("{token}\n")),
            ("crlf", format!("{token}\r\n")),
        ] {
            let path = dir.0.path().join(name);
            fs::write(&path, content).unwrap();
            assert_eq!(read_file(&path), Some(token.clone()), "{name}");
        }
    }

    #[test]
    fn a_token_file_that_is_missing_invalid_or_too_long_is_refused() {
        let dir = TestDir::new();
        assert_eq!(read_file(&dir.0.path().join("missing")), None);
        let token = "a".repeat(64);
        for (name, content) in [
            ("empty", String::new()),
            ("short", "a".repeat(63)),
            ("uppercase", "A".repeat(64)),
            ("slack", format!("{token}\n\n\n")),
            ("long", format!("{token}{token}")),
            ("huge", "a".repeat(1024 * 1024)),
        ] {
            let path = dir.0.path().join(name);
            fs::write(&path, content).unwrap();
            assert_eq!(read_file(&path), None, "{name}");
        }
        let binary = dir.0.path().join("binary");
        fs::write(&binary, [0xff; 64]).unwrap();
        assert_eq!(read_file(&binary), None, "not UTF-8");
    }
}
