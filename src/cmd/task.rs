//! `adj task` — the records the board is a view of.
//!
//! The dashboard is not the only caller. The hub writes back here when it picks a task up
//! ("依頼が届いたら" Step 5 replies to the requester, and for a request from the dashboard
//! the requester is a file rather than a session), and a worker or a script can add one
//! without a browser. So the verbs live here and the HTTP layer calls them, rather than the
//! other way round.

use serde_json::{Value, json};

use super::{Context, Delivered};
use crate::config;
use crate::messaging::{self, Message};
use crate::task::{self, Status, Task};

use std::path::PathBuf;

pub fn dir(ctx: &Context) -> PathBuf {
    task::dir(&messaging::state_dir(), &ctx.repo.slug)
}

fn stamp() -> String {
    messaging::utc_stamp(messaging::now_secs())
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

/// Refuse the two values a person types on the board that the hub later puts on a command
/// line: the worktree name becomes a path and a branch, and the issue URL is quoted as it is.
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
            && std::process::Command::new("git")
                .args(["check-ref-format", "--branch", name])
                .output()
                .is_ok_and(|out| out.status.success());
        if !branchable {
            return Err(format!(
                "git cannot name a branch after this worktree name: {name}"
            ));
        }
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
            return Err(format!("not an issue URL: {url}"));
        }
    }
    Ok(())
}

/// Write a new record, and hand it over if it was created already queued.
pub fn create(ctx: &Context, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    let stamp = stamp();
    let title = derive_title(input).ok_or("a task needs content or a title")?;
    // Before the id is claimed: claiming writes a reservation, and a refusal after it would
    // leave that behind.
    check_typed_values(input)?;
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
    task.worktree = task.worktree.as_deref().map(resolved_worktree);
    task.order = next_order(ctx);

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
    let path = config::expand_home(path);
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
    let dir = dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // Beside the record, and not named `.json`, so the listing never reads it as a task.
    let path = dir.join(format!("{id}.lock"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    file.lock()
        .map_err(|e| format!("cannot lock {}: {e}", path.display()))?;
    Ok(file)
}

pub fn update(ctx: &Context, id: &str, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    let lock = lock_task(ctx, id)?;
    let mut task = task::load(&dir(ctx), id)?;
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

/// `gh pr view --json state -q .state` prints one word. Anything else is not guessed at.
fn parse_pr_state(stdout: &str) -> PrState {
    match stdout.trim() {
        "OPEN" => PrState::Open,
        "CLOSED" => PrState::Closed,
        "MERGED" => PrState::Merged,
        other => PrState::Unreadable(format!("gh answered {other:?}")),
    }
}

/// How long a whole refresh may spend waiting on `gh`. The hub asks in the block it starts
/// with, and the MCP server answers one request at a time, so a `gh` that hangs — no network,
/// a login prompt — would hold up every other answer in that block with it. One deadline for
/// the lot rather than one per PR, so the wait does not grow with the number of records.
const GH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// How many `gh` run at once. Enough that a few dozen records answer in a few round trips;
/// few enough that GitHub does not read the burst as abuse and refuse some of them, which
/// would come back as PRs nobody can read.
const GH_AT_ONCE: usize = 8;

/// Ask `gh` about one pull request.
///
/// From the main checkout, so a record that holds a bare number rather than a URL is read
/// against this repository rather than whichever directory the caller happens to be in.
fn ask_pr_state(main: &str, pr: &str, deadline: std::time::Instant) -> PrState {
    // A value that starts with '-' would reach `gh` as a flag.
    if pr.starts_with('-') {
        return PrState::Unreadable(format!("not a pull request: {pr}"));
    }
    let mut child = match std::process::Command::new("gh")
        .args(["pr", "view", pr, "--json", "state", "-q", ".state"])
        .current_dir(main)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return PrState::Unreadable(format!("cannot run gh: {e}")),
    };
    // Polled rather than waited on: what `gh` prints here is a word or an error line, far
    // short of filling a pipe, so it can sit unread until the process is done.
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return PrState::Unreadable(format!(
                    "gh did not answer within the {}s a refresh allows",
                    GH_TIMEOUT.as_secs()
                ));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return PrState::Unreadable(format!("cannot wait for gh: {e}"));
            }
        }
    }
    let out = match child.wait_with_output() {
        Ok(out) => out,
        Err(e) => return PrState::Unreadable(format!("cannot read gh: {e}")),
    };
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return PrState::Unreadable(if said.is_empty() {
            format!("gh exited with {}", out.status)
        } else {
            said
        });
    }
    parse_pr_state(&String::from_utf8_lossy(&out.stdout))
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

/// Bring the records up to date with their pull requests: every one that has a `pr` and is
/// not finished is asked about, and the ones whose PR was merged are moved to `done`.
///
/// Nothing else is changed. A PR still open is still in review; one closed without merging
/// may have been replaced by another, which only a person knows; one `gh` cannot read is
/// not evidence of anything. Those are returned for the caller to report.
pub fn refresh(ctx: &Context) -> Result<Vec<Checked>, String> {
    let dir = dir(ctx);
    let candidates: Vec<Task> = task::list(&dir)
        .into_iter()
        .filter(|t| !matches!(t.status, Status::Done | Status::Cancelled))
        .filter(|t| t.pr.is_some())
        .collect();
    // A few at a time rather than one by one: each answer is a round trip to GitHub, and
    // the board and the hub's first block both wait on the whole.
    let deadline = std::time::Instant::now() + GH_TIMEOUT;
    let mut states: Vec<PrState> = Vec::with_capacity(candidates.len());
    for batch in candidates.chunks(GH_AT_ONCE) {
        std::thread::scope(|scope| {
            let asks: Vec<_> = batch
                .iter()
                .map(|t| {
                    let pr = t.pr.as_deref().unwrap_or_default();
                    let main = ctx.repo.main.as_str();
                    scope.spawn(move || ask_pr_state(main, pr, deadline))
                })
                .collect();
            states.extend(asks.into_iter().map(|ask| {
                ask.join()
                    .unwrap_or_else(|_| PrState::Unreadable("the check panicked".to_string()))
            }));
        });
    }

    let mut checked = Vec::new();
    for (task, state) in candidates.into_iter().zip(states) {
        if state != PrState::Merged {
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
    Ok(checked)
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
        let worktree = config::expand_home(worktree).to_string_lossy().to_string();
        input["worktree"] = json!(worktree);
        input["handOver"] = json!(false);
    }
    if let Some(title) = args.title {
        input["title"] = json!(title);
    }
    let (task, handed) = create(&ctx, &input)?;
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
    let (task, handed) = update(&ctx, args.id, &input)?;
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
        let path = config::expand_home(path);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_taken_verbatim_when_present() {
        let input = json!({ "title": "explicit title", "body": "first line\nsecond line" });
        assert_eq!(derive_title(&input).as_deref(), Some("explicit title"));
    }

    #[test]
    fn title_is_derived_from_the_first_non_empty_line_of_the_body() {
        let input =
            json!({ "body": "\n\n  Fix the flaky network retry logic  \nand more details" });
        assert_eq!(
            derive_title(&input).as_deref(),
            Some("Fix the flaky network retry logic")
        );
    }

    #[test]
    fn title_is_capped_at_eighty_characters() {
        let long_line = "a".repeat(120);
        let input = json!({ "body": long_line });
        let derived = derive_title(&input).expect("derived");
        assert_eq!(derived.len(), 80);
    }

    /// A form sends the stop point whether or not one was picked, and nothing picked is the
    /// default rather than a refusal.
    #[test]
    fn a_stop_point_left_blank_is_the_default_and_anything_unknown_is_refused() {
        for blank in [json!({}), json!({"stopAt": null}), json!({"stopAt": ""})] {
            assert!(check_typed_values(&blank).is_ok(), "{blank}");
            let filled = with_defaults(&blank, "t", "20260922T000000Z").unwrap();
            assert_eq!(filled["stopAt"], "plan", "{blank}");
        }
        assert_eq!(
            with_defaults(&json!({"stopAt": "all"}), "t", "20260922T000000Z").unwrap()["stopAt"],
            "all"
        );
        for bad in [json!({"stopAt": "verify"}), json!({"stopAt": 1})] {
            assert!(check_typed_values(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_text_field_is_cleared_by_null_or_empty_and_refused_as_anything_else() {
        assert_eq!(
            text_field("pr", &json!("https://x/pull/1"))
                .unwrap()
                .as_deref(),
            Some("https://x/pull/1")
        );
        assert_eq!(text_field("pr", &json!(null)).unwrap(), None);
        assert_eq!(text_field("note", &json!("")).unwrap(), None);
        assert_eq!(
            text_field("instruction", &json!("優先して実装してください"))
                .unwrap()
                .as_deref(),
            Some("優先して実装してください")
        );
        assert_eq!(text_field("instruction", &json!("")).unwrap(), None);
        assert_eq!(text_field("instruction", &json!(null)).unwrap(), None);
        for bad in [json!(42), json!(true), json!(["a"]), json!({"a": 1})] {
            assert!(text_field("pr", &bad).is_err(), "{bad} was taken");
        }
    }

    #[test]
    fn a_pr_state_is_read_from_gh_s_one_word_and_nothing_else_is_guessed_at() {
        assert_eq!(parse_pr_state("MERGED\n"), PrState::Merged);
        assert_eq!(parse_pr_state("OPEN\n"), PrState::Open);
        assert_eq!(parse_pr_state("CLOSED"), PrState::Closed);
        for odd in ["", "merged", "{\"state\":\"MERGED\"}", "DRAFT"] {
            assert!(
                matches!(parse_pr_state(odd), PrState::Unreadable(_)),
                "{odd:?} was read as a state"
            );
        }
    }

    /// A value that would reach `gh` as a flag is refused before `gh` is run at all.
    #[test]
    fn a_pr_that_looks_like_a_flag_is_not_handed_to_gh() {
        assert!(matches!(
            ask_pr_state(".", "--web", std::time::Instant::now()),
            PrState::Unreadable(why) if why.contains("--web")
        ));
    }

    #[test]
    fn empty_title_and_body_produce_nothing() {
        let input = json!({ "title": "   ", "body": "   \n\n  " });
        assert!(derive_title(&input).is_none());
    }

    /// Only a backlog task is said to be kept in the backlog; a task further along is not
    /// sent back there by an update that did not hand it over.
    #[test]
    fn a_task_not_handed_over_is_described_by_where_it_is() {
        assert_eq!(
            not_handed_line(Status::Backlog, "hub").as_deref(),
            Some("Kept in the backlog. Nothing is in hub's inbox yet.")
        );
        let queued = not_handed_line(Status::Queued, "hub").expect("a queued task gets a line");
        assert!(!queued.contains("backlog"), "{queued}");
        assert!(queued.contains("hub"), "{queued}");
        for status in [
            Status::Dispatched,
            Status::Pr,
            Status::Done,
            Status::Cancelled,
        ] {
            assert_eq!(not_handed_line(status, "hub"), None, "{}", status.as_str());
        }
    }
}
