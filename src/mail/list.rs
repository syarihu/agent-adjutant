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

pub(super) fn list(root: &Path, slug: &str) -> Vec<Entry> {
    let dir = inbox_dir(root, slug);
    // Before answering, put back anything an ack was interrupted half way through. This is
    // the one place that reads the whole directory, so it is the one place that can see it.
    put_back_abandoned(&dir);
    let Ok(read) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut names: Vec<String> = vec![];
    // Markers of messages that were read, with when. Collected in the same pass as the names.
    let mut markers: std::collections::HashMap<String, std::time::SystemTime> =
        std::collections::HashMap::new();
    for e in read.flatten() {
        if !e.path().is_file() {
            continue;
        }
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if let Some(of) = name.strip_prefix(SEEN) {
            if let Ok(at) = e.metadata().and_then(|m| m.modified()) {
                markers.insert(of.to_string(), at);
            }
        } else if !name.starts_with('.') {
            names.push(name);
        }
    }
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(&name)).unwrap_or_default();
            // A marker older than the message belongs to an earlier message of that name.
            let seen = markers.get(&name).is_some_and(|marked| {
                dir.join(&name)
                    .metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|sent| *marked >= sent)
            });
            let header = |key: &str| header_value(&text, key).unwrap_or_default();
            Entry {
                subject: header("subject"),
                from: header("from"),
                worktree: header_value(&text, "worktree").filter(|path| !path.is_empty()),
                kind: header("kind"),
                at: header_value(&text, "at")
                    .filter(|at| !at.is_empty())
                    .or_else(|| stamp_of_name(&name)),
                seen,
                name,
            }
        })
        .collect()
}
