//! `adj task` — the records the board is a view of.
//!
//! The dashboard is not the only caller. The hub writes back here when it picks a task up
//! ("When a request arrives" Step 5 replies to the requester, and for a request from the dashboard
//! the requester is a file rather than a session), and a worker or a script can add one
//! without a browser. So the verbs live here and the HTTP layer calls them, rather than the
//! other way round.

use serde_json::{Value, json};

use super::{Context, Delivered};
use crate::kernel::config;
use crate::messaging::Message;
use crate::task::{self, PrRef, PrStatus, Status, Task};

use std::path::{Path, PathBuf};

pub fn dir(ctx: &Context) -> PathBuf {
    task::dir(&ctx.state, &ctx.repo.slug)
}

fn stamp() -> String {
    crate::infra::clock::utc_stamp(crate::infra::clock::now_secs())
}

/// Derive a card title from the input title or the first non-empty line of the body.
fn derive_title(input: &Value) -> Option<String> {
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
pub(super) fn check_worktree_name(name: &str) -> Result<(), String> {
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
fn check_typed_values(input: &Value) -> Result<(), String> {
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
        serde_json::from_value::<task::StopAt>(json!(stop_at))
            .map_err(|_| format!("no such stop point: {stop_at} (plan, diff or all)"))?;
    } else if input
        .get("stopAt")
        .is_some_and(|v| !v.is_null() && !v.is_string())
    {
        return Err(format!("no such stop point: {}", input["stopAt"]));
    }
    if let Some(executor) = typed("executor") {
        task::Executor::parse(executor)
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
fn check_url(url: &str, what: &str) -> Result<(), String> {
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
fn check_parent(parent: &str) -> Result<(), String> {
    if crate::kernel::brief::is_key(parent) {
        return Ok(());
    }
    check_url(parent, "a task")
}

/// Write a new record, and hand it over if it was created already queued.
pub fn create(ctx: &Context, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    let stamp = stamp();
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
    let url = string(input, "issueUrl").filter(|u| task::fetchable_issue(u));
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
    } else if let Some(name) = url.as_deref().filter(|_| reads).and_then(task::issue_ref) {
        // The issue could not be read: keep the record under a name that says which issue it
        // is, and let the first successful read replace it.
        pending = true;
        name
    } else {
        return Err("a task needs content or a title".to_string());
    };
    let id = task::claim_id(&dir(ctx), &stamp, &title)?;
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
    task::save(&dir(ctx), &task)?;
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
fn resolved_worktree(path: &str) -> String {
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

/// One of a record's text fields as an update gives it. `null` and `""` clear it; anything
/// that is not a string is refused rather than read as "clear" — `{"pr": 42}` from a mistaken
/// caller would otherwise wipe the URL it meant to set.
fn text_field(key: &str, value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(v) if v.is_empty() => Ok(None),
        Value::String(v) => Ok(Some(v.clone())),
        other => Err(format!("{key} has to be a string or null, not {other}")),
    }
}

/// Change a record, and hand it over if this is the change that queued it.
/// Hold the write lock of one task record until the returned handle is dropped.
///
/// A change is a read of the whole record and a write of the whole record, and two of them
/// at once — the hub updating a task while a gate for it is answered on the board — would
/// each write back what they read, and the later would undo the earlier. An advisory lock
/// on an open file, like the dispatch lock: the system lets it go if its holder dies, so
/// there is nothing to clear by hand. Held only across a load and a save, so waiting on it
/// is short.
pub fn lock_task(ctx: &Context, id: &str) -> Result<std::fs::File, String> {
    if !task::is_plain_id(id) {
        return Err(format!("no such task: {id}"));
    }
    // Beside the record, and not named `.json`, so the listing never reads it as a task.
    crate::infra::fs::lock(&dir(ctx).join(format!("{id}.lock")))
}

pub fn update(ctx: &Context, id: &str, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    update_checked(ctx, id, input, |_| Ok(()))
}

/// `update`, refusing when `check` says so about the record as it is *under the lock*: a
/// check made before the lock is taken can pass for two callers at once.
pub fn update_checked(
    ctx: &Context,
    id: &str,
    input: &Value,
    check: impl FnOnce(&Task) -> Result<(), String>,
) -> Result<(Task, Option<Delivered>), String> {
    let lock = lock_task(ctx, id)?;
    let mut task = task::load(&dir(ctx), id)?;
    check(&task)?;
    let was = task.status;

    if let Some(status) = string(input, "status") {
        task.status = Status::parse(&status).ok_or(format!("no such status: {status}"))?;
    }
    if let Some(order) = input.get("order").and_then(Value::as_u64) {
        task.order = order as u32;
    }
    // Set by the hub when a person approved a task that asked to be confirmed first, so that
    // being turned away for a slot afterwards does not put the same question to them again.
    if let Some(auto_start) = input.get("autoStart").and_then(Value::as_bool) {
        task.auto_start = auto_start;
    }
    if let Some(executor) = string(input, "executor") {
        task.executor = task::Executor::parse(&executor)
            .ok_or(format!("no such executor: {executor} (worker or jules)"))?;
    }
    let pr_before = task.pr.clone();
    for (key, field) in [
        ("worktree", &mut task.worktree),
        ("issue", &mut task.issue),
        ("pr", &mut task.pr),
        // The branching point the hub decided, for a task whose record did not bring one:
        // `adj jules start` reads it after the hub may have restarted.
        ("base", &mut task.base),
        ("julesSession", &mut task.jules_session),
        ("julesBy", &mut task.jules_by),
        ("note", &mut task.note),
        ("instruction", &mut task.instruction),
    ] {
        if let Some(value) = input.get(key) {
            // An explicit `null` clears; an absent key leaves it alone. Without the
            // distinction there is no way to take back a worktree the hub wrote down. An
            // empty string clears too, since a command line has no way to say `null` — and a
            // "waiting for a slot" note has to go once the worker starts.
            *field = text_field(key, value)?;
        }
    }
    // What the board has brought to the hub belongs to the PR it read. Another PR starts over:
    // kept, the count would leave a new PR at the limit before its first review.
    if task.pr != pr_before {
        task.announced.clear();
        task.relay_rounds = 0;
        // Likewise what the last refresh read: it described the old PR.
        task.pr_status = None;
    }
    // Only a worktree given in this update: one already stored was resolved when it was
    // given, against the directory of the command that gave it, and re-resolving it here
    // would read it against wherever this update happens to be run from.
    if input.get("worktree").is_some() {
        task.worktree = task.worktree.as_deref().map(resolved_worktree);
    }
    task.updated_at = stamp();
    task::save(&dir(ctx), &task)?;
    drop(lock);

    // Handing over is a *transition*, not a status: re-sending on every save would put one
    // task in the inbox once for every time somebody dragged its card.
    // Not when the hub is the one queueing it: the inbox it would land in is its own. That is
    // a resumed worker turned away for a slot, whose record was `dispatched` or `pr`.
    let hand = input
        .get("handOver")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let handed = if hand && was != Status::Queued && task.status == Status::Queued {
        Some(hand_over(ctx, &task)?)
    } else {
        None
    };
    Ok((task, handed))
}

/// Put the task in the hub's inbox and poke its tab — the same delivery `adj send` performs,
/// through the same code, so waking and notifying cannot drift between the two callers.
pub fn hand_over(ctx: &Context, task: &Task) -> Result<Delivered, String> {
    let message = Message {
        from: "dashboard".to_string(),
        // Deliberately none. The sender is a person at a browser, not a worktree, and a
        // `worktree:` header here would name whichever directory the server was started in
        // — which the hub would then act on as if a worker had reported from it.
        worktree: None,
        kind: "request".to_string(),
        subject: task.title.clone(),
        body: task::render_request(task),
    };
    super::deliver_to_hub(ctx, &message)
}

/// Ask the hub to start the next queued task if a worker slot is free.
///
/// For the one case the hub's own procedure cannot see: a worker that died without sending
/// `done`. Its slot came free and nothing woke the hub to say so. A message rather than a
/// bare wake, because a hub that is woken and finds its inbox empty goes straight back to
/// waiting — and one that is not running should find this waiting when it starts.
pub fn nudge(ctx: &Context) -> Result<Delivered, String> {
    let message = Message {
        from: "dashboard".to_string(),
        // None, for the reason `hand_over` gives.
        worktree: None,
        kind: "next".to_string(),
        subject: "start the next queued task if a worker slot is free".to_string(),
        body: String::new(),
    };
    super::deliver_to_hub(ctx, &message)
}

/// What GitHub says about a pull request a record points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    /// Closed without being merged. The work may have gone on in another PR, so this is not
    /// read as "done" or as "cancelled": a person says which.
    Closed,
    Merged,
    /// `gh` could not say: not installed, not signed in, no such PR, or an answer this does
    /// not recognise. Why, in `gh`'s own words where it gave any.
    Unreadable(String),
}

impl PrState {
    fn as_str(&self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Closed => "closed",
            PrState::Merged => "merged",
            PrState::Unreadable(_) => "unreadable",
        }
    }
}

/// How long a whole refresh may spend waiting on `gh`. The hub asks in the block it starts
/// with, and the MCP server answers one request at a time, so a `gh` that hangs — no network,
/// a login prompt — would hold up every other answer in that block with it. One deadline for
/// the lot rather than one per PR, so the wait does not grow with the number of records.
const GH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// How long reading one issue may take. A person is waiting on the command or the click, and
/// an issue is one request, so this is shorter than a whole refresh's allowance.
const ISSUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Run `gh` from `main` and return what it printed, or the reason it failed.
fn gh_output(main: &str, args: &[&str], deadline: std::time::Instant) -> Result<String, String> {
    let run = crate::infra::gh::run(Some(main), args, deadline)?;
    if run.ok {
        return Ok(run.stdout);
    }
    Err(if run.stderr.is_empty() {
        "gh exited without succeeding".to_string()
    } else {
        run.stderr
    })
}

/// Read one issue through `gh`, from the main checkout so a URL on another host is still
/// resolved with this machine's `gh` login.
pub(super) fn read_issue(main: &str, url: &str) -> Result<task::IssueSnapshot, String> {
    // A value that starts with '-' would reach `gh` as a flag.
    if url.starts_with('-') {
        return Err(format!("not an issue: {url}"));
    }
    let deadline = std::time::Instant::now() + ISSUE_TIMEOUT;
    let json = gh_output(
        main,
        // `--` so the URL is only ever a positional, whatever it starts with.
        &["issue", "view", "--json", "title,body", "--", url],
        deadline,
    )?;
    task::snapshot_from_gh(&json, url, &stamp())
}

/// The title of the issue at `url` when a task record of one of these hubs already holds it:
/// the snapshot read for a task whose issue it is, else the title of the task made from it.
/// Looked at before `gh` is asked, so an issue the board has already read is not read again.
pub(super) fn known_title(root: &Path, slugs: &[String], url: &str) -> Option<String> {
    let tasks = slugs
        .iter()
        .flat_map(|slug| task::list(&task::dir(root, slug)));
    let mut fallback = None;
    for t in tasks {
        if let Some(snapshot) = t.issue_snapshot.as_ref().filter(|s| s.url == url) {
            return Some(snapshot.title.clone()).filter(|t| !t.is_empty());
        }
        if fallback.is_none()
            && !t.title_pending
            && t.issue_url.as_deref() == Some(url)
            && !t.title.is_empty()
        {
            fallback = Some(t.title.clone());
        }
    }
    fallback
}

/// Read the task's issue and keep its title and body on the record.
///
/// `gh` runs with no lock held, since it can take seconds; the record is read again under the
/// lock and written only if it still points at the issue that was read, the way `refresh`
/// treats a merged PR. On failure the record is left as it was, an earlier snapshot included.
pub fn fetch_issue(ctx: &Context, id: &str) -> Result<Task, String> {
    let dir = dir(ctx);
    let before = task::load(&dir, id)?;
    let url = task::issue_to_fetch(&before)
        .ok_or("no GitHub issue to read")?
        .to_string();
    let snapshot = read_issue(&ctx.repo.main, &url)?;
    let _lock = lock_task(ctx, id)?;
    let mut now = task::load(&dir, id)?;
    if task::issue_to_fetch(&now) != Some(url.as_str()) {
        return Err("the task's issue changed while it was being read".to_string());
    }
    // A title made from the URL gives way to the issue's own on the first read.
    if now.title_pending && !snapshot.title.is_empty() {
        now.title = snapshot.title.clone();
        now.title_pending = false;
    }
    now.issue_snapshot = Some(snapshot);
    now.updated_at = stamp();
    task::save(&dir, &now)?;
    Ok(now)
}

/// Read the issue of a task that has just started and has none kept. A failure is reported
/// on stderr and nothing more: the command that got here did what it was asked, and the
/// issue can be read again from the board or `adj task fetch-issue`.
///
/// `changed` is whether this very command started the task or moved its issue. Without it,
/// an issue `gh` cannot read would make every later `adj task update --note …` wait on `gh`
/// and print the same failure again.
fn snapshot_if_started(ctx: &Context, task: Task, changed: bool) -> Task {
    if !changed || task::needs_snapshot(&task).is_none() {
        return task;
    }
    match fetch_issue(ctx, &task.id) {
        Ok(read) => read,
        Err(why) => {
            eprintln!("could not read the issue: {why}");
            task
        }
    }
}

/// One record `refresh` looked at, and what came of it.
pub struct Checked {
    pub task: Task,
    pub state: PrState,
    /// Whether this refresh moved the record to `done`. `false` for a merged PR whose record
    /// somebody else changed while `gh` was being asked.
    pub moved: bool,
    /// Why a merged PR's record could not be moved: it was removed while `gh` was being
    /// asked, or it could not be read back or written. One record that fails does not stop
    /// the rest, so the ones already moved are still reported as moved.
    pub failed: Option<String>,
}

/// The records a refresh looks at: every one with a `pr` that is not finished.
pub(super) fn candidates(ctx: &Context) -> Vec<Task> {
    task::list(&dir(ctx))
        .into_iter()
        .filter(|t| !matches!(t.status, Status::Done | Status::Cancelled))
        .filter(|t| t.pr.is_some())
        .collect()
}

/// The pull request each record's `pr` names. A bare number is read against this repository,
/// on the host its origin is on: only when both came from the remote, since a directory name
/// is not a repository GitHub knows, and a number on a host this cannot name would be asked
/// of the wrong one. Origin is asked once, and only if some record holds a bare number.
pub(super) fn pr_refs(ctx: &Context, tasks: &[Task]) -> Vec<Option<PrRef>> {
    let default = std::cell::OnceCell::new();
    tasks
        .iter()
        .map(|t| {
            let pr = t.pr.as_deref()?;
            let default = default.get_or_init(|| {
                (ctx.repo.nwo_source != "dirname")
                    .then(|| crate::kernel::identity::origin_host(&ctx.repo.main))
                    .flatten()
            });
            task::pr_ref(
                pr,
                default.as_deref().map(|host| (host, ctx.repo.nwo.as_str())),
            )
        })
        .collect()
}

/// Why `pr` could not be read as a pull request, for a record whose `pr_refs` entry is `None`.
pub(super) fn unreadable_pr(pr: &str) -> String {
    if task::pr_ref(pr, Some(("github.com", "o/r"))).is_some() {
        format!(
            "a bare pull request number needs a remote on a known host to be read against: {pr}"
        )
    } else {
        format!("not a pull request this can read: {pr}")
    }
}

/// Bring the records up to date with their pull requests: every one that has a `pr` and is
/// not finished is asked about, all in one query, and the ones whose PR was merged are moved
/// to `done`.
///
/// Nothing else is changed. A PR still open is still in review; one closed without merging
/// may have been replaced by another, which only a person knows; one `gh` cannot read is
/// not evidence of anything. Those are returned for the caller to report.
pub fn refresh(ctx: &Context) -> Result<Vec<Checked>, String> {
    let candidates = candidates(ctx);
    let deadline = std::time::Instant::now() + GH_TIMEOUT;
    let refs = pr_refs(ctx, &candidates);
    let readable: Vec<PrRef> = refs.iter().flatten().cloned().collect();
    let mut read = super::gh::read_prs(&readable, deadline).answers.into_iter();
    let answers = candidates
        .iter()
        .zip(&refs)
        .map(|(t, r)| match r {
            Some(_) => read
                .next()
                .unwrap_or_else(|| (PrState::Unreadable("not read".to_string()), None)),
            None => (
                PrState::Unreadable(unreadable_pr(t.pr.as_deref().unwrap_or_default())),
                None,
            ),
        })
        .collect();
    Ok(apply(ctx, candidates, answers))
}

/// Act on what GitHub said about `candidates`, one answer each: a merged PR moves its record
/// to `done`, and any other answer is kept on the record when it differs from what is stored.
/// A PR closed without merging is never cancelled here: the work may have gone on elsewhere.
pub(super) fn apply(
    ctx: &Context,
    candidates: Vec<Task>,
    answers: Vec<(PrState, Option<PrStatus>)>,
) -> Vec<Checked> {
    let dir = dir(ctx);
    let mut checked = Vec::new();
    for (task, (state, summary)) in candidates.into_iter().zip(answers) {
        if state != PrState::Merged {
            // Kept for the board, and nothing else about the record changes. Written only
            // when GitHub's answer differs from what is stored, so a quiet PR costs no write
            // and the page, which redraws when the JSON changes, stays still.
            let task = match summary.filter(|s| task.pr_status.as_ref() != Some(s)) {
                Some(summary) => store_pr_status(ctx, task, summary),
                None => task,
            };
            checked.push(Checked {
                task,
                state,
                moved: false,
                failed: None,
            });
            continue;
        }
        // Read again under the lock: `gh` took a while, and a record somebody moved or
        // pointed at another PR in the meantime is theirs, not this answer's.
        let move_it = || -> Result<(Task, bool), String> {
            let _lock = lock_task(ctx, &task.id)?;
            let mut now = task::load(&dir, &task.id)?;
            let moved = now.pr == task.pr && now.status == task.status;
            if moved {
                now.status = Status::Done;
                now.pr_status = summary.clone().or(now.pr_status);
                now.updated_at = stamp();
                task::save(&dir, &now)?;
            }
            Ok((now, moved))
        };
        checked.push(match move_it() {
            Ok((now, moved)) => Checked {
                task: now,
                state,
                moved,
                failed: None,
            },
            Err(why) => Checked {
                task,
                state,
                moved: false,
                failed: Some(why),
            },
        });
    }
    checked
}

/// Keep what GitHub said about `task`'s PR on its record, if the record still points at that
/// PR once the lock is held. The summary is a cache, so a record that cannot be written is
/// returned as it was rather than failing the refresh. `updatedAt` stays: nobody changed the
/// task, and the board reads that stamp as the last time somebody did.
fn store_pr_status(ctx: &Context, task: Task, summary: PrStatus) -> Task {
    let write = || -> Result<Option<Task>, String> {
        let _lock = lock_task(ctx, &task.id)?;
        let mut now = task::load(&dir(ctx), &task.id)?;
        if now.pr != task.pr || now.pr_status.as_ref() == Some(&summary) {
            return Ok(None);
        }
        now.pr_status = Some(summary);
        task::save(&dir(ctx), &now)?;
        Ok(Some(now))
    };
    match write() {
        Ok(Some(now)) => now,
        Ok(None) => task,
        Err(why) => {
            eprintln!("could not keep the PR summary of {}: {why}", task.id);
            task
        }
    }
}

/// `refresh`'s answer as the MCP tool and the board hand it on: sorted by what happened, so
/// a reader finds what changed without going through what did not.
pub fn refresh_json(checked: &[Checked]) -> Value {
    let entry = |c: &Checked| {
        let mut out = json!({
            "id": c.task.id,
            "title": c.task.title,
            "pr": c.task.pr,
            "status": c.task.status.as_str(),
            "state": c.state.as_str(),
            // Whose turn it is, read from the summary kept on the record; null until one was.
            "turn": c.task.pr_status.as_ref().and_then(task::pr_turn),
        });
        if let PrState::Unreadable(why) = &c.state {
            out["error"] = json!(why);
        }
        if let Some(why) = &c.failed {
            out["error"] = json!(why);
        }
        out
    };
    let with = |pick: fn(&Checked) -> bool| -> Vec<Value> {
        checked.iter().filter(|c| pick(c)).map(entry).collect()
    };
    json!({
        "done": with(|c| c.moved),
        "open": with(|c| c.state == PrState::Open),
        "closed": with(|c| c.state == PrState::Closed),
        "unreadable": with(|c| matches!(c.state, PrState::Unreadable(_))),
        // Merged, but the record changed while `gh` was being asked, so it was left as it is.
        "skipped": with(|c| c.state == PrState::Merged && !c.moved && c.failed.is_none()),
        // Merged, but the record could not be moved. `error` says why.
        "failed": with(|c| c.failed.is_some()),
    })
}

fn next_order(ctx: &Context) -> u32 {
    task::list(&dir(ctx))
        .iter()
        .map(|t| t.order)
        .max()
        .unwrap_or(0)
        + 1
}

/// Fill in what a caller may leave out, so a form can post the fields a person filled and
/// nothing else.
fn with_defaults(input: &Value, id: &str, stamp: &str) -> Result<Value, String> {
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
        fields.insert(
            "stopAt".to_string(),
            json!(task::StopAt::default().as_str()),
        );
    }
    fields.entry("autoStart").or_insert(json!(true));
    fields.entry("status").or_insert(json!("backlog"));
    fields.entry("body").or_insert(json!(""));
    Ok(value)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ── the subcommands ──────────────────────────────────────────────────

pub struct AddArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub kind: &'a str,
    pub done_when: &'a str,
    pub stop_at: &'a str,
    pub executor: &'a str,
    pub issue_url: Option<&'a str>,
    pub base: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub worktree_name: Option<&'a str>,
    pub ask_first: bool,
    pub queue: bool,
    pub waiting_in: Option<&'a str>,
    pub json: bool,
}

pub fn add(args: &AddArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let body = super::read_body(args.body)?;
    let mut input = json!({
        "body": body,
        "kind": args.kind,
        "doneWhen": args.done_when,
        "stopAt": args.stop_at,
        "executor": args.executor,
        "issueUrl": args.issue_url,
        "base": args.base,
        "parent": args.parent,
        "worktreeName": args.worktree_name,
        "autoStart": !args.ask_first,
        "status": if args.queue || args.waiting_in.is_some() { "queued" } else { "backlog" },
    });
    // The hub writing down work it has prepared a worktree for: before it starts the worker,
    // so the brief can carry the id, or after `adj work` turned it away for want of a slot.
    // Queued either way, so a free slot can take it; but not handed over, because the inbox
    // it would land in is the caller's own, and a hub that messages itself is woken mid-turn
    // to be told what it just did.
    if let Some(worktree) = args.waiting_in {
        let worktree = crate::infra::paths::expand_home(worktree)
            .to_string_lossy()
            .to_string();
        input["worktree"] = json!(worktree);
        input["handOver"] = json!(false);
    }
    if let Some(title) = args.title {
        input["title"] = json!(title);
    }
    let (task, handed) = create(&ctx, &input)?;
    let task = snapshot_if_started(&ctx, task, true);
    if args.json {
        println!(
            "{}",
            json!({ "task": task, "handed": handed_json(&handed) })
        );
        return Ok(());
    }
    println!("{} — {}", task.id, task.title);
    say_where_it_went(&ctx, &task, &handed);
    Ok(())
}

pub struct UpdateArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: &'a str,
    pub status: Option<&'a str>,
    pub order: Option<u32>,
    pub worktree: Option<&'a str>,
    pub issue: Option<&'a str>,
    pub pr: Option<&'a str>,
    pub base: Option<&'a str>,
    pub jules_session: Option<&'a str>,
    pub executor: Option<&'a str>,
    pub note: Option<&'a str>,
    pub instruction: Option<&'a str>,
    pub auto_start: Option<bool>,
    pub no_hand_over: bool,
    pub json: bool,
}

pub fn update_cmd(args: &UpdateArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let mut input = json!({});
    let fields = input.as_object_mut().expect("just built");
    // `--note -` reads it from stdin: a note is often text from elsewhere — an error, a
    // comment typed on the board — and does not belong inside quotes on a command line.
    let note = args.note.map(super::dash_is_stdin).transpose()?;
    let instruction = args.instruction.map(super::dash_is_stdin).transpose()?;
    for (key, value) in [
        ("status", args.status),
        ("worktree", args.worktree),
        ("issue", args.issue),
        ("pr", args.pr),
        ("base", args.base),
        ("julesSession", args.jules_session),
        ("executor", args.executor),
        ("note", note.as_deref()),
        ("instruction", instruction.as_deref()),
    ] {
        if let Some(value) = value {
            fields.insert(key.to_string(), json!(value));
        }
    }
    if let Some(order) = args.order {
        fields.insert("order".to_string(), json!(order));
    }
    if let Some(auto_start) = args.auto_start {
        fields.insert("autoStart".to_string(), json!(auto_start));
    }
    if args.no_hand_over {
        fields.insert("handOver".to_string(), json!(false));
    }
    // Read without the lock and `.ok()`: a failed or raced read can only cost one extra fetch
    // attempt, since `needs_snapshot` still guards it.
    let before = task::load(&dir(&ctx), args.id).ok();
    let (task, handed) = update(&ctx, args.id, &input)?;
    let changed = before.is_none_or(|b| {
        !matches!(b.status, Status::Dispatched | Status::Pr)
            || task::issue_to_fetch(&b) != task::issue_to_fetch(&task)
    });
    let task = snapshot_if_started(&ctx, task, changed);
    if args.json {
        println!(
            "{}",
            json!({ "task": task, "handed": handed_json(&handed) })
        );
        return Ok(());
    }
    println!("{} — {} ({})", task.id, task.title, task.status.as_str());
    say_where_it_went(&ctx, &task, &handed);
    Ok(())
}

/// `adj task next`: the queued task a free worker slot should take, and the queued tasks that
/// ask first and have no `dispatch` gate open yet.
pub fn next_cmd(repo: Option<&str>, hub: Option<&str>, as_json: bool) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    // Open gates only: an answered one has gone to the archive, and its answer is the hub's
    // to act on from the inbox.
    let gated: std::collections::HashSet<String> =
        crate::gate::list_of_kind(&super::gate::dir(&ctx), crate::gate::Kind::Dispatch)
            .into_iter()
            .filter_map(|g| g.task)
            .collect();
    let next = task::next(task::list(&dir(&ctx)), &gated);
    if as_json {
        println!("{}", json!(next));
        return Ok(());
    }
    match &next.task {
        Some(t) => println!("next: {} — {}", t.id, t.title),
        None => println!("Nothing queued can be started."),
    }
    for t in &next.needs_dispatch_gate {
        println!("needs a dispatch gate: {} — {}", t.id, t.title);
    }
    Ok(())
}

pub fn list(
    repo: Option<&str>,
    hub: Option<&str>,
    status: Option<&str>,
    worktree: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let wanted = match status {
        Some(text) => Some(Status::parse(text).ok_or(format!("no such status: {text}"))?),
        None => None,
    };
    // Compared resolved: the hub names the worktree as `git worktree list` printed it, and
    // the record holds whatever path it was written with.
    let resolved = |path: &str| {
        let path = crate::infra::paths::expand_home(path);
        path.canonicalize().unwrap_or(path)
    };
    let at = worktree.map(resolved);
    let tasks: Vec<Task> = task::list(&dir(&ctx))
        .into_iter()
        .filter(|t| wanted.is_none_or(|w| t.status == w))
        .filter(|t| {
            at.as_ref()
                .is_none_or(|at| t.worktree.as_deref().map(resolved).as_ref() == Some(at))
        })
        .collect();

    if as_json {
        println!("{}", json!(tasks));
        return Ok(());
    }
    if tasks.is_empty() {
        println!("No tasks for {}.", ctx.repo.nwo);
        return Ok(());
    }
    for task in &tasks {
        println!(
            "{:<10} {:>3}  {}  {}",
            task.status.as_str(),
            task.order,
            task.id,
            task.title
        );
    }
    Ok(())
}

pub fn show(repo: Option<&str>, hub: Option<&str>, id: &str) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let task = task::load(&dir(&ctx), id)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&task).map_err(|e| e.to_string())?
    );
    Ok(())
}

/// `adj task fetch-issue`: read the issue again, whatever the record holds.
pub fn fetch_issue_cmd(
    repo: Option<&str>,
    hub: Option<&str>,
    id: &str,
    as_json: bool,
) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let task = fetch_issue(&ctx, id)?;
    if as_json {
        println!("{}", json!({ "task": task }));
        return Ok(());
    }
    let title = task
        .issue_snapshot
        .as_ref()
        .map_or("", |s| s.title.as_str());
    println!("{} — {}", task.id, title);
    Ok(())
}

pub fn refresh_cmd(repo: Option<&str>, hub: Option<&str>, as_json: bool) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let checked = refresh(&ctx)?;
    if as_json {
        println!("{}", refresh_json(&checked));
        return Ok(());
    }
    if checked.is_empty() {
        println!("No task is waiting on a pull request.");
        return Ok(());
    }
    for c in &checked {
        let what = match (&c.state, c.moved) {
            _ if c.failed.is_some() => format!(
                "merged, but the record could not be moved: {}",
                c.failed.as_deref().unwrap_or_default()
            ),
            (_, true) => "done".to_string(),
            (PrState::Merged, false) => {
                "merged, but the record changed meanwhile; left alone".to_string()
            }
            (PrState::Unreadable(why), _) => format!("unreadable: {why}"),
            (state, _) => format!("{}; left alone", state.as_str()),
        };
        println!(
            "{:<10} {}  {}  {}",
            c.task.status.as_str(),
            c.task.id,
            c.task.pr.as_deref().unwrap_or("-"),
            what
        );
    }
    Ok(())
}

fn handed_json(handed: &Option<Delivered>) -> Value {
    match handed {
        Some(d) => json!({
            "present": d.delivery.present,
            "woken": d.woken,
            "path": d.delivery.path.display().to_string(),
        }),
        None => Value::Null,
    }
}

/// What to say when nothing was put in the hub's inbox, which depends on where the task is
/// now: only a backlog task is "kept in the backlog". A queued one was either handed over
/// earlier or is waiting for a slot the hub itself asked for; anything further along is
/// already shown by its status.
fn not_handed_line(status: Status, hub_name: &str) -> Option<String> {
    match status {
        Status::Backlog => Some(format!(
            "Kept in the backlog. Nothing is in {hub_name}'s inbox yet."
        )),
        Status::Queued => Some(format!("Queued, but not handed to {hub_name} this time.")),
        Status::Dispatched | Status::Pr | Status::Done | Status::Cancelled => None,
    }
}

fn say_where_it_went(ctx: &Context, task: &Task, handed: &Option<Delivered>) {
    let Some(handed) = handed else {
        if let Some(line) = not_handed_line(task.status, &ctx.repo.hub_name) {
            println!("{line}");
        }
        return;
    };
    println!(
        "handed to {}: {}",
        ctx.repo.hub_name,
        handed.delivery.path.display()
    );
    match (handed.delivery.present, handed.woken) {
        (true, true) => println!("Woke the hub; it will pick this up."),
        (true, false) => {
            println!("The hub is running; it will pick this up the next time it checks its inbox.")
        }
        (false, _) => println!(
            "The hub is not running. Waiting in its inbox for the next time it starts (task {}).",
            task.id
        ),
    }
}

/// What `adj task brief` is given.
pub struct BriefArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    /// The task the worker is for. `None` writes the brief of a session with no task.
    pub id: Option<&'a str>,
    pub worktree: &'a str,
    pub base: &'a str,
    pub key: Option<&'a str>,
    pub tracker: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub instruction: Option<&'a str>,
    pub out: Option<&'a str>,
    pub json: bool,
}

/// The Done when the brief says. The worker branches on three phrases only, and a task that
/// goes as far as handling review has opened its PR, so `Review` is written as the PR.
fn brief_done_when(done_when: task::DoneWhen) -> &'static str {
    match done_when {
        task::DoneWhen::Review => task::DoneWhen::Pr.as_prose(),
        other => other.as_prose(),
    }
}

/// A flag value that says something: a blank one is the same as not given.
fn given(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// `adj task brief`: write the worker's `.claude/task-brief.md` from the record and the config.
///
/// The hub used to fill the brief in from a template by hand. Written here, the lines come
/// from the record the board shows, so the two cannot disagree, and the one thing the hub
/// has to get right is the record.
pub fn brief(args: &BriefArgs<'_>) -> Result<(), String> {
    use crate::kernel::brief as text;

    let ctx = super::context(args.repo, args.hub)?;
    let worktree = resolved_worktree(args.worktree);
    let worktree = std::path::Path::new(&worktree);
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    // Read from the worktree rather than taken as an argument: it is the branch the worker
    // will be on, and a typed one is a second answer to a question git already has.
    let branch = crate::infra::git::git(&["symbolic-ref", "-q", "--short", "HEAD"], Some(worktree))
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!(
                "{} is not on a branch: the brief names the branch the worker works on",
                worktree.display()
            )
        })?;
    let config = ctx
        .resolved
        .config
        .clone()
        .unwrap_or_else(|| Value::Object(config::builtin_defaults()));
    let base = args.base.trim();
    if base.is_empty() {
        return Err("--base is empty: pass the commit-ish the worktree was cut from, or -".into());
    }

    let rendered = match args.id {
        Some(id) => {
            let (key_arg, tracker_arg, parent_arg) =
                (given(args.key), given(args.tracker), given(args.parent));
            if let Some(parent) = parent_arg {
                check_parent(parent)?;
            }
            // Written to a line the worker reads, and the hub puts it on a command line.
            if let Some(key) = key_arg.filter(|key| *key != "-" && !text::is_key(key)) {
                return Err(format!("not a tracker key: {key} (like ABC-123)"));
            }
            let record = task::load(&dir(&ctx), id)?;
            if record.executor == task::Executor::Jules {
                return Err(format!(
                    "task {id} is handed to Jules: no worker is started, so no brief is written"
                ));
            }
            // An empty URL is none: `--issue-url ''` stores one.
            let nonblank = |url: &Option<String>| url.clone().filter(|url| !url.trim().is_empty());
            let url = nonblank(&record.issue_url).or_else(|| nonblank(&record.issue));
            let (key, tracker) = match (&url, key_arg, tracker_arg) {
                (_, Some(key), Some(tracker)) => (key.to_string(), tracker.to_string()),
                (None, key, tracker) => (
                    key.unwrap_or("-").to_string(),
                    tracker.unwrap_or("-").to_string(),
                ),
                (Some(url), key, tracker) => {
                    let sources = config
                        .get("taskSources")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let issue_keys = config
                        .get("issueKeys")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    let (read_tracker, read_key) = text::tracker_and_key(url, sources, &issue_keys)
                        .ok_or_else(|| {
                            format!(
                                "cannot tell the tracker and key of {url}: pass --key and --tracker"
                            )
                        })?;
                    (
                        key.map_or(read_key, str::to_string),
                        tracker.map_or(read_tracker, str::to_string),
                    )
                }
            };
            if !["github", "github-project", "jira", "linear", "-"].contains(&tracker.as_str()) {
                return Err(format!(
                    "no such tracker: {tracker} (github, github-project, jira, linear or -)"
                ));
            }
            let verify = config
                .get("verify")
                .and_then(Value::as_array)
                .map(|commands| {
                    commands
                        .iter()
                        .map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let parent = parent_arg
                .map(str::to_string)
                .or(record
                    .parent
                    .clone()
                    .filter(|parent| !parent.trim().is_empty()))
                .unwrap_or_else(|| "-".to_string());
            // The worker fetches the parent from its URL; a bare key says nothing of the tracker.
            // Checked again here because a record written before parents were checked on the way
            // in may hold anything.
            if parent != "-" {
                if crate::kernel::brief::is_key(&parent) {
                    return Err(format!(
                        "the parent task is a key ({parent}): find its URL and pass --parent '<URL>'"
                    ));
                }
                check_url(&parent, "a task")?;
            }
            text::render_task(&text::TaskBrief {
                key,
                title: record.title.clone(),
                tracker,
                url,
                request: record.body.clone(),
                branch: branch.clone(),
                base: base.to_string(),
                parent,
                record: record.id.clone(),
                done_when: brief_done_when(record.done_when).to_string(),
                stop_at: record.stop_at.as_str().to_string(),
                handover: record
                    .instruction
                    .clone()
                    .filter(|note| !note.trim().is_empty())
                    .unwrap_or_else(|| "-".to_string()),
                copilot_review: config
                    .get("copilotReview")
                    .and_then(Value::as_str)
                    .unwrap_or("ask")
                    .to_string(),
                verify,
            })
        }
        None => {
            let instruction = args
                .instruction
                .ok_or("a brief with no --id needs --instruction")?;
            let instruction = super::dash_is_stdin(instruction)?;
            let instruction = match instruction.trim() {
                "" | "-" => text::NO_INSTRUCTION.to_string(),
                _ => instruction,
            };
            text::render_session(&text::SessionBrief {
                branch: branch.clone(),
                base: base.to_string(),
                instruction,
            })
        }
    };

    let path = match args.out {
        Some(out) => crate::infra::paths::expand_home(out),
        None => worktree.join(".claude").join("task-brief.md"),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    // Always written over: the brief is derived, and one left from an earlier start is the
    // stale answer this exists to replace.
    std::fs::write(&path, rendered).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    if args.json {
        println!(
            "{}",
            json!({ "path": path, "task": args.id, "branch": branch })
        );
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
