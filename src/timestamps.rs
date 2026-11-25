//! Timestamps as milliseconds since the Unix epoch, and the UTC calendar dates they fall on: parsing the
//! RFC 3339 timestamps of agent logs, and formatting for people and for day folders.

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Milliseconds since the epoch for an RFC 3339 timestamp such as `2026-11-05T21:17:49.123Z` or with a
/// `+01:00` offset; `None` for anything else.
pub(crate) fn parse(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let digits = |from: usize, len: usize| -> Option<i64> {
        let part = bytes.get(from..from + len)?;
        part.iter().all(u8::is_ascii_digit).then(|| {
            part.iter()
                .fold(0i64, |acc, digit| acc * 10 + i64::from(digit - b'0'))
        })
    };
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut at = 19;
    let mut millis = 0;
    if bytes.get(at) == Some(&b'.') {
        let fraction_start = at + 1;
        at = fraction_start;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let fraction = &text[fraction_start..at];
        if fraction.is_empty() {
            return None;
        }
        let padded = format!("{:0<3}", &fraction[..fraction.len().min(3)]);
        millis = padded.parse::<i64>().ok()?;
    }
    let offset_minutes = match bytes.get(at)? {
        b'Z' | b'z' if at + 1 == bytes.len() => 0,
        sign @ (b'+' | b'-') if at + 6 == bytes.len() && bytes[at + 3] == b':' => {
            let minutes = digits(at + 1, 2)? * 60 + digits(at + 4, 2)?;
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => return None,
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
        - offset_minutes * 60;
    u64::try_from(seconds * 1000 + millis).ok()
}

/// `2026-11-05 21:17 UTC` for milliseconds since the epoch.
pub(crate) fn format(millis: u64) -> String {
    let seconds = (millis / 1000) as i64;
    let (y, m, d) = civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        in_day / 3600,
        in_day % 3600 / 60
    )
}

/// `YYYY-MM-DD` for milliseconds since the epoch, to compare with the day folders.
pub(crate) fn day(millis: u64) -> String {
    format(millis)[..10].to_string()
}

/// `2026-11-05T10:00:05.000Z`, for fixtures that need a log timestamp.
#[cfg(test)]
pub(crate) fn iso(millis: u64) -> String {
    let seconds = millis / 1000;
    let (y, m, d) = civil_from_days((seconds / 86_400) as i64);
    let in_day = seconds % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
        millis % 1000
    )
}
