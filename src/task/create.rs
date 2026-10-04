//! Creating a task record from what a form or a command gives.

use super::*;

use serde_json::{Value, json};

use crate::mail::DeliveryOutcome;
use crate::registry::Context;

/// Derive a card title from the input title or the first non-empty line of the body.
pub(super) fn derive_title(input: &Value) -> Option<String> {
    if let Some(title) = string(input, "title") {
        return Some(title);
    }
    let body = string(input, "body")?;
    let first_line = body.lines().map(str::trim).find(|l| !l.is_empty())?;
    let title: String = first_line.chars().take(80).collect();
    if title.is_empty() { None } else { Some(title) }
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
pub(super) fn check_typed_values(input: &Value) -> Result<(), String> {
    // An empty field is a form left blank, not a value.
    let typed = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    if let Some(name) = typed("worktreeName") {
        check_worktree_name(name)?;
    }
    // Not typed, but refused here all the same: past this point the id is claimed, and a
    // value serde turns away afterwards would leave the reservation behind.
    if let Some(stop_at) = typed("stopAt") {
        serde_json::from_value::<StopAt>(json!(stop_at))
            .map_err(|_| format!("no such stop point: {stop_at} (plan, diff or all)"))?;
    } else if input
        .get("stopAt")
        .is_some_and(|v| !v.is_null() && !v.is_string())
    {
        return Err(format!("no such stop point: {}", input["stopAt"]));
    }
    if let Some(executor) = typed("executor") {
        Executor::parse(executor)
            .ok_or_else(|| format!("no such executor: {executor} (worker or jules)"))?;
    }
    if let Some(url) = typed("issueUrl") {
        check_url(url, "an issue")?;
    }
    // The parent task is typed on the form too, and the procedure quotes it on a command line.
    if let Some(parent) = typed("parent") {
        check_parent(parent)?;
    }
    Ok(())
}

/// Refuse a URL that is not safe to put in quotes on a command line: not an http(s) address
/// with a plausible host, or holding a character the shell would read.
pub fn check_url(url: &str, what: &str) -> Result<(), String> {
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
pub fn check_parent(parent: &str) -> Result<(), String> {
    if crate::kernel::brief::is_key(parent) {
        return Ok(());
    }
    check_url(parent, "a task")
}

/// Write a new record, and hand it over if it was created already queued.
pub fn create(ctx: &Context, input: &Value) -> Result<(Task, Option<DeliveryOutcome>), String> {
    let stamp = store::stamp();
    // Before anything reaches `gh` or the id is claimed: claiming writes a reservation, and a
    // refusal after it would leave that behind.
    check_typed_values(input)?;
    // Likewise a record that will not deserialize: it is refused here, not after a wait on `gh`
    // and a claimed id.
    let mut probe = with_defaults(input, "probe", &stamp)?;
    probe["title"] = json!("probe");
    serde_json::from_value::<Task>(probe).map_err(|e| format!("bad task: {e}"))?;
    // A request that names an issue and says nothing else is a request to hand that issue
    // over: read it now so the card has its title from the start. What the person typed wins.
    // Only for a task that starts an issue: any other kind with no content has nothing to go on.
    let starts = string(input, "kind").is_none_or(|k| k == "start");
    let url = string(input, "issueUrl").filter(|u| fetchable_issue(u));
    let reads = starts && url.is_some() && string(input, "body").is_none();
    let snapshot = match &url {
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
    let title = if let Some(title) = derive_title(input) {
        title
    } else if let Some(snapshot) = &snapshot {
        snapshot.title.clone()
    } else if let Some(name) = url.as_deref().filter(|_| reads).and_then(issue_ref) {
        // The issue could not be read: keep the record under a name that says which issue it
        // is, and let the first successful read replace it.
        pending = true;
        name
    } else {
        return Err("a task needs content or a title".to_string());
    };
    let id = store::claim_id(ctx, &stamp, &title)?;
    let mut defaults = with_defaults(input, &id, &stamp)?;
    defaults["title"] = json!(title);
    // An instruction to this function rather than part of the record.
    let hand = defaults
        .as_object_mut()
        .and_then(|fields| fields.remove("handOver"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut task: Task = serde_json::from_value(defaults).map_err(|e| format!("bad task: {e}"))?;
    // Only keys read from disk are carried; a caller's unknown keys are dropped, as before.
    task.extra.clear();
    task.worktree = task.worktree.as_deref().map(resolved_worktree);
    task.order = next_order(ctx);
    // Set here and not through the input, which `with_defaults` strips of both.
    task.issue_snapshot = snapshot;
    task.title_pending = pending;

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
pub fn resolved_worktree(path: &str) -> String {
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

/// Fill in what a caller may leave out, so a form can post the fields a person filled and
/// nothing else.
pub(super) fn with_defaults(input: &Value, id: &str, stamp: &str) -> Result<Value, String> {
    let mut value = input.clone();
    let fields = value.as_object_mut().ok_or("expected an object")?;
    fields.insert("id".to_string(), json!(id));
    // Only a fetch writes the snapshot: a caller's copy would be text nobody read from the
    // issue, shown on the board as if somebody had.
    fields.remove("issueSnapshot");
    // Likewise the PR summary: only a refresh read it from GitHub.
    fields.remove("prStatus");
    // And the title's flag: only `create` knows that the issue could not be read.
    fields.remove("titlePending");
    fields.insert("createdAt".to_string(), json!(stamp));
    fields.insert("updatedAt".to_string(), json!(stamp));
    fields.entry("kind").or_insert(json!("start"));
    fields.entry("doneWhen").or_insert(json!("pr"));
    // `null` and `""` too: a form sends the field whether or not anything was picked in it.
    if fields
        .get("stopAt")
        .is_none_or(|v| v.is_null() || v.as_str() == Some(""))
    {
        fields.insert("stopAt".to_string(), json!(StopAt::default().as_str()));
    }
    fields.entry("autoStart").or_insert(json!(true));
    fields.entry("status").or_insert(json!("backlog"));
    fields.entry("body").or_insert(json!(""));
    Ok(value)
}

pub(super) fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
