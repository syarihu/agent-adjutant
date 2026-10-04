use super::*;

/// Leave `message` for the hub. Returns where it landed and whether anyone was there to see
/// it arrive.
pub fn send(slug: &str, hub_name: &str, message: &Message) -> Result<Delivery, String> {
    let status = hub_status(slug, hub_name);
    let dir = inbox_dir(slug);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let kind = if message.kind.is_empty() {
        "report"
    } else {
        &message.kind
    };
    let stamp = utc_stamp(now_secs());
    let staged = stage(&dir, &render_message(message))?;
    let claimed = claim_link(&staged, &dir, |seq| match seq {
        0 => format!("{stamp}-{kind}.md"),
        seq => format!("{stamp}-{seq}-{kind}.md"),
    });
    // The staging name has served its purpose either way. Leaving it behind would be
    // invisible to `list`, which is worse than a stray file: an inbox that quietly grows.
    let _ = std::fs::remove_file(&staged);
    Ok(Delivery {
        path: claimed?,
        present: status.present,
    })
}

pub fn render_message(message: &Message) -> String {
    let subject = if message.subject.is_empty() {
        message
            .body
            .lines()
            .next()
            .unwrap_or("(no subject)")
            .to_string()
    } else {
        message.subject.clone()
    };
    // Left out entirely when there is none, rather than written empty. An empty value reads
    // as "no worktree" to a person and as an empty path to a program, and the two disagree
    // the moment something tries to act on it.
    let worktree = match message
        .worktree
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        Some(path) => format!("worktree: {}\n", one_line(path)),
        None => String::new(),
    };
    format!(
        "---\nfrom: {}\n{worktree}kind: {}\nsubject: {}\nat: {}\n---\n\n{}\n",
        one_line(&message.from),
        one_line(if message.kind.is_empty() {
            "report"
        } else {
            &message.kind
        }),
        one_line(&subject),
        utc_stamp(now_secs()),
        message.body.trim_end()
    )
}
