// ── titles ───────────────────────────────────────────────────────────

pub(super) fn default_title(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

/// Newlines are the dangerous character here: one inside an AppleScript string literal
/// submits the half-typed command in the new tab. Quotes and backslashes survive, because
/// `applescript_literal` escapes them properly.
pub fn sanitise_title(title: &str, fallback: String) -> String {
    let collapsed: Vec<&str> = title.split_whitespace().collect();
    let title = collapsed.join(" ");
    let title = if title.is_empty() { fallback } else { title };
    if display_width(&title) <= 30 {
        return title;
    }
    let mut out = String::new();
    for c in title.chars() {
        if display_width(&out) + char_width(c) > 28 {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn char_width(c: char) -> usize {
    // East Asian Wide and Fullwidth take two cells. The ranges here are the ones a task
    // title actually lands in: CJK, kana, and fullwidth punctuation.
    let c = c as u32;
    let wide = (0x1100..=0x115F).contains(&c)
        || (0x2E80..=0x303E).contains(&c)
        || (0x3041..=0x33FF).contains(&c)
        || (0x3400..=0x4DBF).contains(&c)
        || (0x4E00..=0x9FFF).contains(&c)
        || (0xA000..=0xA4CF).contains(&c)
        || (0xAC00..=0xD7A3).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        || (0xFE30..=0xFE6F).contains(&c)
        || (0xFF00..=0xFF60).contains(&c)
        || (0xFFE0..=0xFFE6).contains(&c)
        || (0x1F300..=0x1FAFF).contains(&c);
    if wide { 2 } else { 1 }
}

pub(super) fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}
