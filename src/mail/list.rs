use super::*;

/// The leading `YYYYMMDDTHHMMSSZ` of an inbox file name.
fn stamp_of_name(name: &str) -> Option<String> {
    let stamp = name.get(..16)?;
    let shaped = stamp.bytes().enumerate().all(|(i, b)| match i {
        8 => b == b'T',
        15 => b == b'Z',
        _ => b.is_ascii_digit(),
    });
    shaped.then(|| stamp.to_string())
}

pub fn list(slug: &str) -> Vec<Entry> {
    let dir = inbox_dir(slug);
    // Before answering, put back anything an ack was interrupted half way through. This is
    // the one place that reads the whole directory, so it is the one place that can see it.
    put_back_abandoned(&dir);
    let Ok(read) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(&name)).unwrap_or_default();
            let header = |key: &str| header_value(&text, key).unwrap_or_default();
            Entry {
                subject: header("subject"),
                from: header("from"),
                worktree: header_value(&text, "worktree").filter(|path| !path.is_empty()),
                kind: header("kind"),
                at: header_value(&text, "at")
                    .filter(|at| !at.is_empty())
                    .or_else(|| stamp_of_name(&name)),
                name,
            }
        })
        .collect()
}
