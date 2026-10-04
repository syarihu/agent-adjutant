use super::*;

#[test]
fn stamp_is_utc_and_sorts() {
    assert_eq!(utc_stamp(0), "19700101T000000Z");
    assert_eq!(utc_stamp(1_767_225_600), "20260101T000000Z");
    assert_eq!(utc_stamp(1_757_306_100), "20250908T043500Z");
    assert!(utc_stamp(1) < utc_stamp(2));
}

#[test]
fn stamp_handles_leap_days() {
    // 2024-02-29T12:00:00Z — a year that is a leap year despite the /100 rule biting in
    // the neighbouring centuries.
    assert_eq!(utc_stamp(1_709_208_000), "20240229T120000Z");
    // 2000-02-29T00:00:00Z — the /400 exception.
    assert_eq!(utc_stamp(951_782_400), "20000229T000000Z");
}
