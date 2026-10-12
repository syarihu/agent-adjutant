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

#[test]
fn a_time_from_github_becomes_a_stamp() {
    assert_eq!(
        stamp_from_iso("2026-10-10T05:06:07Z").as_deref(),
        Some("20261010T050607Z")
    );
    assert_eq!(
        stamp_from_iso("2026-10-10T05:06:07.123Z").as_deref(),
        Some("20261010T050607Z")
    );
    // The same instant, so the same stamp as `utc_stamp` makes.
    assert_eq!(
        stamp_from_iso("2026-01-01T00:00:00Z"),
        Some(utc_stamp(1_767_225_600))
    );
}

#[test]
fn a_time_in_another_shape_is_not_a_stamp() {
    for text in [
        "",
        "2026-10-10",
        "2026-10-10T05:06:07",
        "2026-10-10T05:06:07+09:00",
        "2026-10-10T05:06:07.Z",
        "2026-10-10T05:06:07.1xZ",
        "2026-10-10T05:06:07Zjunk",
        "abcd-ef-ghTij:kl:mnZ",
        "2026/10/10T05:06:07Z",
    ] {
        assert_eq!(stamp_from_iso(text), None, "{text:?}");
    }
}
