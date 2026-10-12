use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `YYYYMMDDTHHMMSSZ`. UTC, and said so in the name: these strings sort, appear in filenames
/// and get copied into issues, and a local time with no offset in it is the kind of thing
/// that is wrong for half the year without anyone noticing.
pub fn utc_stamp(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let secs = epoch_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// An adj stamp out of the time GitHub writes, `YYYY-MM-DDTHH:MM:SSZ` (a fractional second
/// allowed), so every time a record holds is one format and compares as a string. Any other
/// shape, an offset other than `Z` included, is `None`: guessing a zone would be wrong for
/// somebody.
pub fn stamp_from_iso(iso: &str) -> Option<String> {
    let b = iso.as_bytes();
    // The shape of `YYYY-MM-DDTHH:MM:SS`, digit or the one character that stands there.
    const SHAPE: &[u8; 19] = b"dddd-dd-ddTdd:dd:dd";
    if b.len() < 20
        || !b[..19].iter().zip(SHAPE).all(|(&c, &want)| match want {
            b'd' => c.is_ascii_digit(),
            want => c == want,
        })
    {
        return None;
    }
    let rest = &iso[19..];
    let zone = match rest.strip_prefix('.') {
        Some(fraction) => fraction
            .strip_suffix('Z')
            .filter(|f| !f.is_empty() && f.bytes().all(|c| c.is_ascii_digit())),
        None => (rest == "Z").then_some(rest),
    };
    zone?;
    Some(format!(
        "{}{}{}T{}{}{}Z",
        &iso[0..4],
        &iso[5..7],
        &iso[8..10],
        &iso[11..13],
        &iso[14..16],
        &iso[17..19]
    ))
}

/// Howard Hinnant's days-from-civil, inverted. Shifting the era to start in March makes the
/// leap day the last day of the year, which is what removes the month-length special cases.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests;
