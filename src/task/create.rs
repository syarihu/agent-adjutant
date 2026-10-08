//! Creating a task record from what a form or a command gives.

use super::*;

use crate::mail::DeliveryOutcome;
use crate::registry::Context;

/// Derive a card title from the given title or the first non-empty line of the body. The whole
/// line is kept: every place that shows a title wraps it rather than shortening it.
pub(super) fn derive_title(title: Option<&str>, body: &str) -> Option<String> {
    if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        return Some(title.to_string());
    }
    let first_line = body.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(first_line.to_string())
}

/// A worktree name as the hub will use it: a path and a branch, so its characters are
/// limited and git has to agree it can name a branch.
pub fn check_worktree_name(name: &str) -> Result<(), String> {
    let charset = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !charset {
        return Err(format!(
            "a worktree name may only use letters, digits, '.', '_' and '-': {name}"
        ));
    }
    // Whether it can name a branch is git's to say, not a list kept here: saved, a name git
    // refuses would sit in the queue until the hub failed to create its branch. Asked with
    // the name alone, which `branchPattern` puts after a prefix — a name that fails on its
    // own fails there too. A leading '-' is refused first so git cannot read it as a flag.
    let branchable = !name.starts_with('-')
        && crate::infra::git::git(&["check-ref-format", "--branch", name], None)
            .is_ok_and(|out| out.status.success());
    if !branchable {
        return Err(format!(
            "git cannot name a branch after this worktree name: {name}"
        ));
    }
    Ok(())
}

/// Refuse the two values a person types on the board that the hub later puts on a command
/// line: the worktree name becomes a path and a branch, and the issue and parent task URLs are
/// quoted as they are.
/// Checked here, where they come in, rather than in every command the procedures write — an
/// apostrophe in either would close the quote around it and run the rest as shell.
pub(super) fn check_typed(new: &NewTask) -> Result<(), String> {
    // An empty field is a form left blank, not a value.
    fn typed(value: &Option<String>) -> Option<&str> {
        value.as_deref().filter(|v| !v.is_empty())
    }
    if let Some(name) = typed(&new.worktree_name) {
        check_worktree_name(name)?;
    }
    if let Some(url) = typed(&new.issue_url) {
        check_url(url, "an issue")?;
    }
    // The parent task is typed on the form too, and the procedure quotes it on a command line.
    if let Some(parent) = typed(&new.parent) {
        check_parent(parent)?;
    }
    Ok(())
}

/// Refuse a URL that is not safe to put in quotes on a command line: not an http(s) address
/// with a plausible host, or holding a character the shell would read.
pub(super) fn check_url(url: &str, what: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    // The host is what is left of the authority once a port is taken off. Checked as a
    // name rather than as "some text before the path", which `https://:8080/` passed.
    // A port, when there is one, is digits.
    let authority = rest
        .and_then(|r| r.split(['/', '?', '#']).next())
        .unwrap_or("");
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let named = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && port.is_none_or(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let plain = !url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "'\"`$\\;&|<>(){}".contains(c));
    if !named || !plain {
        return Err(format!("not {what} URL: {url}"));
    }
    Ok(())
}

/// A parent task as the board takes it: a URL, or a key the hub turns into one before it
/// writes the brief. Anything else is refused, since it is quoted on a command line.
pub(super) fn check_parent(parent: &str) -> Result<(), String> {
    if crate::kernel::brief::is_key(parent) {
        return Ok(());
    }
    check_url(parent, "a task")
}

/// Write a new record, and hand it over if it was created already queued and `hand` says so.
/// `hand` is an instruction to this function, not a field of the task.
pub fn create(
    ctx: &Context,
    new: NewTask,
    hand: bool,
) -> Result<(Task, Option<DeliveryOutcome>), String> {
    let stamp = store::stamp();
    // Before anything reaches `gh` or the id is claimed: claiming writes a reservation, and a
    // refusal after it would leave that behind.
    check_typed(&new)?;
    // A request that names an issue and says nothing else is a request to hand that issue
    // over: read it now so the card has its title from the start. What the person typed wins.
    // Only for a task that starts an issue: any other kind with no content has nothing to go on.
    let starts = new.kind == Kind::Start;
    let url = new
        .issue_url
        .as_deref()
        .map(str::trim)
        .filter(|u| fetchable_issue(u));
    let reads = starts && url.is_some() && new.body.trim().is_empty();
    let snapshot = match url {
        Some(url) if reads => match read_issue(&ctx.repo.main, url) {
            Ok(read) => Some(read).filter(|s| !s.title.is_empty()),
            Err(why) => {
                eprintln!("could not read the issue: {why}");
                None
            }
        },
        _ => None,
    };
    let mut pending = false;
    let title = if let Some(title) = derive_title(new.title.as_deref(), &new.body) {
        title
    } else if let Some(snapshot) = &snapshot {
        snapshot.title.clone()
    } else if let Some(name) = url.filter(|_| reads).and_then(issue_ref) {
        // The issue could not be read: keep the record under a name that says which issue it
        // is, and let the first successful read replace it.
        pending = true;
        name
    } else {
        return Err("a task needs content or a title".to_string());
    };
    let id = store::claim_id(ctx, &stamp, &title)?;
    let task = Task {
        id,
        kind: new.kind,
        title,
        body: new.body,
        issue_url: new.issue_url,
        done_when: new.done_when,
        stop_at: new.stop_at,
        executor: new.executor,
        base: new.base,
        parent: new.parent,
        worktree_name: new.worktree_name,
        auto_start: new.auto_start,
        order: next_order(ctx),
        status: new.status,
        worktree: new.worktree.as_deref().map(resolved_worktree),
        issue: None,
        pr: None,
        jules_session: None,
        jules_by: None,
        relayed: Vec::new(),
        announced: Vec::new(),
        relay_rounds: 0,
        note: None,
        instruction: None,
        gate_answered_at: None,
        // Only a fetch writes the snapshot, and only `create` knows the title came from the
        // issue URL because the issue could not be read.
        issue_snapshot: snapshot,
        title_pending: pending,
        pr_status: None,
        pr_turn_at: None,
        created_at: stamp.clone(),
        updated_at: stamp,
        extra: Default::default(),
    };

    // Written before the message is sent, and never the other way round: the record is what
    // the hub checks when it is about to act, so a message that arrived first would name a
    // task nothing can look up.
    store::save(ctx, &task)?;
    let handed = match task.status {
        Status::Queued if hand => Some(hand_over(ctx, &task)?),
        _ => None,
    };
    Ok((task, handed))
}

/// A worktree path as `git worktree list` prints it: absolute, symlinks resolved. The board
/// matches a task to its worker by this string, and `./wt` or `/tmp/…` against git's
/// `/private/tmp/…` would read as a worker that is not there. A path that does not exist
/// (yet) is resolved through the nearest part of it that does, with the rest put back on,
/// so that it is absolute — against the directory of the command giving it, not of some
/// later one — and matches what git prints once the worktree is created there.
pub(super) fn resolved_worktree(path: &str) -> String {
    let path = crate::infra::paths::expand_home(path);
    let mut current = std::path::absolute(&path).unwrap_or(path);
    // Until it stops changing: stepping back over a part that does not exist can land on
    // one that does — a symlink, say — which only the next pass resolves. Bounded, since a
    // path has only so many parts to settle.
    for _ in 0..16 {
        let next = resolve_once(&current);
        if next == current {
            break;
        }
        current = next;
    }
    current.to_string_lossy().to_string()
}

/// One pass of `resolved_worktree`: the longest leading part that exists is resolved by the
/// system, `..` and symlinks and all. What follows does not exist yet, so there is nothing
/// to follow through it: `..` there steps back up and `.` is dropped, which is what git does
/// when it creates it.
fn resolve_once(absolute: &std::path::Path) -> std::path::PathBuf {
    use std::path::{Component, PathBuf};
    let parts: Vec<Component> = absolute.components().collect();
    for split in (1..=parts.len()).rev() {
        let head: PathBuf = parts[..split].iter().collect();
        let Ok(mut resolved) = head.canonicalize() else {
            continue;
        };
        for part in &parts[split..] {
            match part {
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::Normal(name) => resolved.push(name),
                _ => {}
            }
        }
        return resolved;
    }
    absolute.to_path_buf()
}

fn next_order(ctx: &Context) -> u32 {
    list(&ctx.state, &ctx.repo.slug)
        .iter()
        .map(|t| t.order)
        .max()
        .unwrap_or(0)
        + 1
}
