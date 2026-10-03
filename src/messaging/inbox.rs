use super::*;

// ── messages ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Message {
    /// Who is speaking. A worker's session name, or whatever the sending agent calls itself.
    pub from: String,
    /// Where it is speaking *from*: the absolute path of the sender's own worktree.
    ///
    /// `from` cannot carry this. It is free text, chosen by the sender, and names a session
    /// at best — so a hub acting on a request to close a tab and remove a worktree was
    /// acting on a path typed into the body. This is derived from where the sender actually
    /// is, and it is filled in for every kind: which worktree a report came from is worth
    /// the same line as which worktree a finished task is in, and a format whose headers
    /// depend on the kind is one every reader eventually mis-parses.
    ///
    /// `None` when the sender could not be placed in a worktree at all, and then the header
    /// is left out rather than written empty.
    pub worktree: Option<String>,
    /// `report`, `question`, `answer`, `ack`, `done`, or anything the two sides agree on.
    pub kind: String,
    /// The one line a human will actually read.
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub path: PathBuf,
    pub present: bool,
}

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

/// Newlines in a header value would let a body forge extra headers, and a colon in a value
/// is harmless but confusing. Collapsing whitespace handles both.
pub(super) fn one_line(text: &str) -> String {
    let collapsed: Vec<&str> = text.split_whitespace().collect();
    collapsed.join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub subject: String,
    pub from: String,
    /// The sender's worktree, for the messages that carry one. `None` covers both "sent
    /// from outside a worktree" and "written before this header existed": an inbox outlives
    /// an upgrade, and a listing that failed on the messages already in it would strand
    /// them.
    pub worktree: Option<String>,
    pub kind: String,
    /// When it was sent: the `at` header, or the stamp its file name starts with for a
    /// message that has none. `None` when neither reads as a stamp.
    pub at: Option<String>,
}

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

pub fn header_value(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    for line in lines {
        if line == "---" {
            return None;
        }
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

pub fn read(slug: &str, name: &str) -> Result<String, String> {
    let path = safe_join(&inbox_dir(slug), name)?;
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Move one message out of the way. The hub calls this once it has filed the report.
///
/// The archive is where a mishandled report is found again, so an archived name is never
/// reused: two messages acked in the same second are two files, not one file and one loss.
///
/// The message is *taken* before it is filed, not after. Filing first and unlinking second
/// is the order that loses one: two acks of a name each file a copy, the first unlink frees
/// the name, a send in the same second claims it, and the second unlink deletes that new
/// message — which nothing filed. So the inbox name is claimed in one step, by renaming it
/// onto a name only this call knows. A second acker's rename finds nothing and says so.
///
/// Between the two steps the message is real but hidden, which is a state this has to be
/// able to come back from: the name it was taken from is written into the holding name, and
/// `list` puts back anything it finds there that is too old to be in flight.
pub fn ack(slug: &str, name: &str) -> Result<PathBuf, String> {
    let inbox = inbox_dir(slug);
    let from = safe_join(&inbox, name)?;
    let dir = archive_dir(slug);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let held = hold(&inbox, name)?;
    if let Err(e) = std::fs::rename(&from, &held) {
        let _ = std::fs::remove_file(&held);
        return match e.kind() {
            std::io::ErrorKind::NotFound => Err(format!("no message called {name} is waiting")),
            _ => Err(format!("cannot move {}: {e}", from.display())),
        };
    }
    // From here the message exists only under the holding name, so it is unlinked from
    // there only once something else holds it. An ack that cannot file *and* cannot put
    // back leaves it where the sweep below will find it, rather than deleting it to keep
    // the directory tidy.
    match claim_link(&held, &dir, |seq| numbered(name, seq)) {
        Ok(to) => {
            let _ = std::fs::remove_file(&held);
            Ok(to)
        }
        Err(e) => match claim_link(&held, &inbox, |seq| numbered(name, seq)) {
            Ok(back) => {
                let _ = std::fs::remove_file(&held);
                Err(format!(
                    "{e} — the message is back in the inbox as {}",
                    back.file_name().unwrap_or_default().to_string_lossy()
                ))
            }
            Err(_) => Err(format!(
                "{e} — the message is held at {} and will be put back",
                held.display()
            )),
        },
    }
}

/// The prefix a message wears while it is being acked. A dotfile, so `list` does not offer
/// it and `safe_join` will not open it — it is mid-move, not waiting — and it carries the
/// name it came from so that a move interrupted half way can be undone.
pub(super) const HOLDING: &str = ".acking-";

/// How long a message may be held before `list` decides nobody is coming back for it. An
/// ack holds one across two syscalls, so anything this old is from a process that died.
pub(super) const HELD_STALE_SECS: u64 = 60;

fn hold(inbox: &Path, name: &str) -> Result<PathBuf, String> {
    for attempt in 0..CLAIM_ATTEMPTS {
        let path = inbox.join(format!("{HOLDING}{}-{attempt}-{name}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot hold a message in {} after {CLAIM_ATTEMPTS} tries",
        inbox.display()
    ))
}

/// Put back anything an interrupted ack left holding.
///
/// Without this, a process that died between taking a message and filing it left the only
/// copy under a name nothing lists, nothing reads and nothing acks — a report that exists
/// and cannot be reached, which is worse than the overwrite this whole arrangement replaced.
/// Age is what distinguishes an abandoned hold from one in flight, and the margin is wide:
/// an ack holds a message for two syscalls.
fn put_back_abandoned(inbox: &Path) {
    let Ok(read) = std::fs::read_dir(inbox) else {
        return;
    };
    for entry in read.flatten() {
        let held = entry.path();
        let Some(rest) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_prefix(HOLDING).map(str::to_string))
        else {
            continue;
        };
        // `<pid>-<attempt>-<the name it came from>`.
        let Some(name) = rest.splitn(3, '-').nth(2).map(str::to_string) else {
            continue;
        };
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|at| at.elapsed().map(|d| d.as_secs()).unwrap_or(0) > HELD_STALE_SECS)
            .unwrap_or(false);
        if !too_old {
            continue;
        }
        if claim_link(&held, inbox, |seq| numbered(&name, seq)).is_ok() {
            let _ = std::fs::remove_file(&held);
        }
    }
}

/// A message name comes from an agent, so it is untrusted input used as a path. Anything
/// with a separator in it is rejected outright rather than sanitised — a name that needed
/// sanitising was not one of ours.
fn safe_join(dir: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(format!("invalid message name: {name}"));
    }
    Ok(dir.join(name))
}

/// How many names are tried before a claim gives up. Same-second sends are normal (a
/// worker filing two findings at once), so the counter is not an edge case to skip; a
/// thousand of them in one second is not a collision but a runaway.
pub(super) const CLAIM_ATTEMPTS: usize = 1000;

/// Give `staged` a second name in `dir`, the first one `name_for` offers that is free.
///
/// Two properties have to hold at once, and one primitive gives both. `hard_link` refuses
/// an existing name instead of replacing it, so two senders racing for the same second
/// cannot both win — which `exists()` followed by a write cannot promise, because the
/// answer is already stale by the time it is acted on. And because the name being claimed
/// points at a file that is *already written in full*, nobody can read half a message.
///
/// `rename` would give the second property and lose the first: it replaces silently, which
/// is exactly the overwrite being ruled out here. So the staged file gets linked, not moved,
/// and the caller unlinks the staging name afterwards.
fn claim_link(
    staged: &Path,
    dir: &Path,
    name_for: impl Fn(usize) -> String,
) -> Result<PathBuf, String> {
    for seq in 0..CLAIM_ATTEMPTS {
        let path = dir.join(name_for(seq));
        match std::fs::hard_link(staged, &path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot find an unused name in {} after {CLAIM_ATTEMPTS} tries",
        dir.display()
    ))
}

/// `report.md` with `seq` worked into it: `report-2.md`. Suffixing the stem rather than the
/// whole name keeps the extension where a reader (and an editor) expects it.
pub(super) fn numbered(name: &str, seq: usize) -> String {
    if seq == 0 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}-{seq}.{ext}"),
        _ => format!("{name}-{seq}"),
    }
}
