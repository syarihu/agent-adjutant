//! `adj jules` — hand a task's approved plan to Jules, and ask how that is going.
//!
//! The worker is the caller. It plans in its worktree as it always does, and once the plan
//! gate is answered it writes the design out for Jules and runs `adj jules start`, which
//! starts the session and writes its id onto the task record. From there the record is what
//! the board and the hub follow: the session id is the only thing about Jules kept locally.

use serde_json::{Value, json};

use super::task as tasks;
use crate::jules;
use crate::task;

pub struct StartArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: &'a str,
    /// Where the prompt is: a file, or `-` for stdin.
    pub prompt: &'a str,
    pub base: Option<&'a str>,
    pub json: bool,
}

pub fn start(args: &StartArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    // Read before anything is started: a session that exists with nobody recorded as its
    // starter would have its review comments passed on in the wrong name, and a lookup that
    // hangs after the session is created would leave it running unrecorded.
    let by = github_login(&ctx.repo.main);
    // Held from the checks to the record, across the call that creates the session. Two starts
    // for one task at once would otherwise both find it without a session and both create one,
    // and the record would keep whichever wrote last.
    let lock = tasks::lock_task(&ctx, args.id)?;
    let mut task = task::load(&tasks::dir(&ctx), args.id)?;
    // Handed over by the worker of a task in progress, and by nobody else. The board follows a
    // session only while its task is in progress or in review, so one started for a task still
    // in the backlog or the queue would run with nothing watching it.
    if task.status != task::Status::Dispatched {
        return Err(format!(
            "{} is {}, not in progress: a task goes to Jules from its worker, after the hub has dispatched it",
            task.id,
            task.status.as_str()
        ));
    }
    // Who implements was decided when the task was written down, and a worker-implemented
    // task handed to Jules as well would be implemented twice.
    if task.executor != task::Executor::Jules {
        return Err(format!(
            "{} is to be implemented by its worker; `adj task update --id {} --executor jules` first if Jules should do it",
            task.id, task.id
        ));
    }
    // A second session for one task is two pull requests for one change, and the first one
    // would be forgotten: the record keeps one id.
    if let Some(session) = &task.jules_session {
        return Err(format!(
            "{} is already with Jules (session {session}); clear it with `adj task update --id {} --jules-session ''` to start another",
            task.id, task.id
        ));
    }
    // Given explicitly, the branch is taken as it is. The task's own base is what was typed on
    // the board, where `origin/feature/x` is as likely as `feature/x`; Jules wants the name
    // GitHub has, so that one is checked against this checkout's remote-tracking refs.
    let base = match (args.base, task.base.as_deref()) {
        (Some(given), _) => given.to_string(),
        (None, Some(stored)) => branch_on_github(&ctx.repo.main, &ctx.repo.nwo, stored),
        (None, None) => {
            return Err("which branch should Jules start from? pass --base (the branch this worktree was cut from, without origin/)".to_string());
        }
    };
    let base = base.as_str();
    let prompt = read_prompt(args.prompt)?;
    let session = jules::create(
        &ctx.settings.jules_key,
        &ctx.repo.nwo,
        base,
        &task.title,
        &prompt,
    )?;
    // Written as soon as the session exists, under the lock taken above. If this fails the
    // session is still running, so the error says which one it is rather than leaving it to be
    // found on jules.google.com.
    task.jules_session = Some(session.id.clone());
    // Who started it, as far as `gh` can say. Jules acts on comments by that person only, so a
    // comment passed on in anybody else's name would be posted and ignored.
    task.jules_by = by;
    task.updated_at = crate::messaging::utc_stamp(crate::messaging::now_secs());
    task::save(&tasks::dir(&ctx), &task).map_err(|e| {
        format!(
            "Jules started session {} but the task record could not be updated: {e}",
            session.id
        )
    })?;
    drop(lock);
    if args.json {
        println!("{}", json!({ "task": task.id, "session": session }));
        return Ok(());
    }
    println!("{} — handed to Jules as session {}", task.id, session.id);
    if let Some(url) = &session.url {
        println!("{url}");
    }
    Ok(())
}

pub struct ShowArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: Option<&'a str>,
    pub session: Option<&'a str>,
    pub json: bool,
}

/// How a session is doing, named by its task or by its own id.
pub fn show(args: &ShowArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let session_id = match (args.session, args.id) {
        (Some(session), _) => session.to_string(),
        (None, Some(id)) => task::load(&tasks::dir(&ctx), id)?
            .jules_session
            .ok_or(format!("{id} has not been handed to Jules"))?,
        (None, None) => return Err("name the task with --id or the session with --session".into()),
    };
    let session = jules::get(&ctx.settings.jules_key, &session_id)?;
    if args.json {
        println!("{}", serde_json::to_value(&session).unwrap_or(Value::Null));
        return Ok(());
    }
    println!("{} {}", session.id, session.state);
    for line in [&session.url, &session.pr].into_iter().flatten() {
        println!("{line}");
    }
    Ok(())
}

/// The branch as GitHub names it, for a base written the way `git worktree add` takes it.
///
/// `origin/` is taken off only when this checkout says it is the remote's prefix: a
/// remote-tracking ref of that name exists, and no branch on the remote is itself called
/// `origin/…`. Anything the checkout cannot vouch for is left as it was written, for the API to
/// accept or refuse — stripping blindly would turn a real branch named `origin/x` into `x`.
///
/// Only when the checkout is a clone of the repository the session is for. `--repo` names the
/// repository without moving the command into its checkout, and another repository's refs say
/// nothing about this one's branches.
fn branch_on_github(main: &str, nwo: &str, base: &str) -> String {
    let Some(rest) = base.strip_prefix("origin/") else {
        return base.to_string();
    };
    if crate::repo::name_with_owner(main).0 != nwo {
        return base.to_string();
    }
    let known = |name: &str| {
        crate::repo::git(
            &[
                "-C",
                main,
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{name}"),
            ],
            None,
        )
        .is_ok_and(|out| out.status.success())
    };
    if known(&format!("origin/{base}")) || !known(base) {
        base.to_string()
    } else {
        rest.to_string()
    }
}

/// The GitHub account `gh` is signed in as, or `None` when it cannot say.
///
/// Given up on after `LOGIN_TIMEOUT`: `gh` waiting on a login prompt or a network that has
/// gone should not hold up a command that has more to do.
pub fn github_login(main: &str) -> Option<String> {
    let mut child = std::process::Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .current_dir(main)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + LOGIN_TIMEOUT;
    // Polled rather than waited on: what `gh` prints here is one short line, far short of
    // filling a pipe, so it can sit unread until the process is done.
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child
        .wait_with_output()
        .ok()
        .filter(|o| o.status.success())?;
    let login = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!login.is_empty()).then_some(login)
}

/// How long `github_login` waits for `gh`.
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The prompt, from a file or from stdin. The design is dozens of lines and does not belong
/// on a command line, where a quote in it would end the argument early.
fn read_prompt(from: &str) -> Result<String, String> {
    let text = match from {
        "-" => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            buf
        }
        path => {
            let path = crate::config::expand_home(path);
            std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        }
    };
    if text.trim().is_empty() {
        return Err("the prompt is empty".to_string());
    }
    Ok(text)
}

// ── the board's view of the sessions ─────────────────────────────────

/// How long an answer about a session is reused. The board polls every two seconds; the
/// session's state changes over minutes. Asking once in this long keeps the badge current
/// enough to act on without sending the API a request per poll per open tab.
const FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(45);

/// The last answer about each session, and whether a question is already out.
///
/// The board asks from a thread of its own, never from the request that serves the page: an
/// API that is slow to answer should make a badge a little stale, not the whole board.
#[derive(Default)]
pub struct Watch {
    seen: std::sync::Mutex<std::collections::HashMap<String, Seen>>,
}

struct Seen {
    at: std::time::Instant,
    asking: bool,
    answer: Option<Result<jules::Session, String>>,
}

impl Watch {
    /// What the board shows for a task: the last answer about its session, or `None` for a
    /// task Jules is not working on. Asks again in the background when the answer is old.
    ///
    /// Only for a task still in progress or in review: once it is done nobody reads the badge,
    /// and a session that is left alone does not change.
    pub fn look(
        self: &std::sync::Arc<Self>,
        ctx: &super::Context,
        key: &crate::config::Hook,
        task: &Value,
    ) -> Option<Value> {
        let session = task.get("julesSession")?.as_str()?.to_string();
        let status = task.get("status").and_then(Value::as_str)?;
        if !matches!(status, "dispatched" | "pr") {
            return None;
        }
        let task_id = task.get("id")?.as_str()?.to_string();
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(session.clone()).or_insert(Seen {
            at: std::time::Instant::now(),
            asking: false,
            answer: None,
        });
        let stale = entry.answer.is_none() || entry.at.elapsed() >= FRESH_FOR;
        if stale && !entry.asking {
            entry.asking = true;
            let watch = std::sync::Arc::clone(self);
            let ctx = ctx.clone();
            let key = key.clone();
            let asked = session.clone();
            std::thread::spawn(move || watch.ask(&ctx, &key, &task_id, &asked));
        }
        Some(match &entry.answer {
            None => json!({ "session": session, "checking": true }),
            Some(Ok(found)) => json!({
                "session": session,
                "state": found.state,
                "url": found.url,
                "pr": found.pr,
                "working": working(&found.state),
                "age": entry.at.elapsed().as_secs(),
            }),
            Some(Err(why)) => json!({
                "session": session,
                "error": why,
                "age": entry.at.elapsed().as_secs(),
            }),
        })
    }

    fn ask(&self, ctx: &super::Context, key: &crate::config::Hook, task_id: &str, session: &str) {
        let answer = jules::get(key, session);
        if let Ok(found) = &answer
            && let Err(e) = follow(ctx, task_id, found)
        {
            eprintln!("adj serve: could not record the pull request of {task_id}: {e}");
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = seen.get_mut(session) {
            entry.at = std::time::Instant::now();
            entry.asking = false;
            entry.answer = Some(answer);
        }
    }
}

/// Whether Jules is doing something with the session right now. A session goes back to
/// `IN_PROGRESS` while it answers a comment on its pull request, and to `COMPLETED` once it
/// has pushed, so this flips more than once in a task's life.
pub fn working(state: &str) -> bool {
    matches!(state, "QUEUED" | "PLANNING" | "IN_PROGRESS")
}

/// The first time a session is seen with a pull request: write it onto the task, move the
/// card to review, and tell the hub, which has the PR's description to rewrite.
///
/// Keyed on the record having no PR yet, so it happens once however many times the session
/// finishes — it finishes again after every round of comments it answers.
fn follow(ctx: &super::Context, task_id: &str, session: &jules::Session) -> Result<(), String> {
    let Some(pr) = &session.pr else {
        return Ok(());
    };
    let task = task::load(&tasks::dir(ctx), task_id)?;
    if task.pr.is_some() || task.jules_session.as_deref() != Some(session.id.as_str()) {
        return Ok(());
    }
    let mut change = json!({ "pr": pr });
    if task.status == task::Status::Dispatched {
        change["status"] = json!("pr");
    }
    let (task, _) = tasks::update(ctx, task_id, &change)?;
    let message = crate::messaging::Message {
        from: "jules".to_string(),
        // None, for the reason `task::hand_over` gives: this comes from no worktree.
        worktree: None,
        kind: "jules-pr".to_string(),
        subject: task.title.clone(),
        body: format!(
            "## task        {}\n## pr          {pr}\n## session     {}\n",
            task.id, session.id
        ),
    };
    super::deliver_to_hub(ctx, &message).map(|_| ())
}
