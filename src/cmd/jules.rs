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

/// `adj jules findings`: the review comments that could be passed on.
pub fn findings_cmd(
    repo: Option<&str>,
    hub: Option<&str>,
    id: &str,
    as_json: bool,
) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let found = findings(&ctx, id)?;
    if as_json {
        println!("{}", json!(found));
        return Ok(());
    }
    for f in &found {
        let place = f.line.map_or(f.path.clone(), |l| format!("{}:{l}", f.path));
        let mark = if f.relayed { " (passed on)" } else { "" };
        println!("{} {place}{mark}", f.id);
        println!("  {}", f.text.lines().next().unwrap_or(""));
    }
    Ok(())
}

/// `adj jules relay`: pass the chosen review comments on to Jules.
pub fn relay_cmd(
    repo: Option<&str>,
    hub: Option<&str>,
    id: &str,
    comments: &[String],
    note: Option<&str>,
) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let note = note.map(super::dash_is_stdin).transpose()?;
    let done = relay(&ctx, id, comments, note.as_deref())?;
    println!("passed {} comment(s) on to Jules", comments.len());
    if let Some(url) = done["comment"].as_str().filter(|u| !u.is_empty()) {
        println!("{url}");
    }
    Ok(())
}

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
        // Nothing that changes by itself goes in here — an age in seconds, say. The page redraws
        // whenever the state it polls differs from the last, and a field that ticks would have
        // it redraw every two seconds.
        Some(match &entry.answer {
            None => json!({ "session": session, "checking": true }),
            Some(Ok(found)) => json!({
                "session": session,
                "state": found.state,
                "url": found.url,
                "pr": found.pr,
                "working": working(&found.state),
            }),
            Some(Err(why)) => json!({
                "session": session,
                "error": why,
            }),
        })
    }

    /// Forget the sessions no card showed on this poll: finished tasks, sessions replaced.
    /// One still being asked about is kept, so its answer has somewhere to land. Without this
    /// the map grows with every session the board has ever shown.
    pub fn keep_only(&self, shown: &std::collections::HashSet<String>) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|session, entry| entry.asking || shown.contains(session));
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
///
/// All of it under the task's lock, decided on the record as it is now rather than as it was
/// when the question to Jules went out: the answer can take seconds, and by then the task may
/// have been finished, pulled back, or given another session, whose card this PR is not.
///
/// The hub is told first and the record written after. The other order loses the message for
/// good when delivery fails: the record already has its PR, so no later poll gets this far
/// again. This order at worst tells the hub twice, when the write fails after a delivery.
fn follow(ctx: &super::Context, task_id: &str, session: &jules::Session) -> Result<(), String> {
    let Some(pr) = &session.pr else {
        return Ok(());
    };
    let lock = tasks::lock_task(ctx, task_id)?;
    let mut task = task::load(&tasks::dir(ctx), task_id)?;
    let waiting = task.pr.is_none()
        && task.jules_session.as_deref() == Some(session.id.as_str())
        && matches!(task.status, task::Status::Dispatched | task::Status::Pr);
    if !waiting {
        return Ok(());
    }
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
    // Posted under the lock, so no second poll can post it again; the hub is woken and the
    // person told after the lock is let go, since those run commands that may not return.
    let posted = super::post_to_hub(ctx, &message)?;
    task.pr = Some(pr.clone());
    if task.status == task::Status::Dispatched {
        task.status = task::Status::Pr;
    }
    task.updated_at = crate::messaging::utc_stamp(crate::messaging::now_secs());
    let saved = task::save(&tasks::dir(ctx), &task);
    drop(lock);
    posted.follow_up(ctx, true);
    saved.map(|_| ())
}

// ── review comments passed on to Jules ───────────────────────────────

/// Who writes the review comments worth passing on, when the repository names no review
/// bots. Jules does not act on another bot's comments, only on those of the person who
/// started it, so these have to be restated in that person's name.
const DEFAULT_REVIEWERS: [&str; 1] = ["coderabbitai[bot]"];

/// One inline review comment on the task's pull request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub author: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    pub url: String,
    /// What Jules is given: the reviewer's own prompt for an agent when there is one,
    /// otherwise the comment without its hidden and folded parts.
    pub text: String,
    /// Already passed on.
    pub relayed: bool,
}

/// The review comments on the task's pull request from the repository's review bots, oldest
/// first. Replies are left out: a thread is passed on by its first comment.
pub fn findings(ctx: &super::Context, id: &str) -> Result<Vec<Finding>, String> {
    let task = task::load(&tasks::dir(ctx), id)?;
    let pr = task
        .pr
        .as_deref()
        .ok_or(format!("{id} has no pull request yet"))?;
    let number = pr_number(pr).ok_or(format!("not a pull request URL: {pr}"))?;
    let reviewers = reviewers(ctx);
    let out = std::process::Command::new("gh")
        .args([
            "api",
            &format!("repos/{}/pulls/{number}/comments", ctx.repo.nwo),
            "--paginate",
            "--jq",
            ".[] | {id, path, line, original_line, body, html_url, in_reply_to_id, user: .user.login}",
        ])
        .current_dir(&ctx.repo.main)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run gh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh could not list the review comments: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let listed = String::from_utf8_lossy(&out.stdout);
    Ok(parse_findings(&listed, &reviewers, &task.relayed))
}

/// Post the chosen comments to the pull request as one comment in the person's own name, for
/// Jules to act on, and note them as passed on.
pub fn relay(
    ctx: &super::Context,
    id: &str,
    chosen: &[String],
    note: Option<&str>,
) -> Result<Value, String> {
    if chosen.is_empty() {
        return Err("choose at least one comment to pass on".to_string());
    }
    // Held from the check to the save, across both round trips to GitHub. Two relays of one
    // comment — a double click, the board and a shell at once — would otherwise both find it
    // not yet passed on and both post it. Waiting a few seconds on a button somebody pressed
    // is the cheaper failure.
    let lock = tasks::lock_task(ctx, id)?;
    let all = findings(ctx, id)?;
    let mut picked = Vec::new();
    for want in chosen {
        let found = all
            .iter()
            .find(|f| &f.id == want)
            .ok_or(format!("no review comment {want} on this pull request"))?;
        if found.relayed {
            return Err(format!("comment {want} has already been passed on"));
        }
        picked.push(found);
    }
    let task = task::load(&tasks::dir(ctx), id)?;
    let pr = task
        .pr
        .clone()
        .ok_or(format!("{id} has no pull request yet"))?;
    let body = relay_body(&picked, note);
    // On stdin: the text is the reviewers' and the person's, and neither belongs on a
    // command line.
    let mut child = std::process::Command::new("gh")
        .args(["pr", "comment", &pr, "--body-file", "-"])
        .current_dir(&ctx.repo.main)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run gh: {e}"))?;
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().ok_or("gh has no stdin")?;
        stdin
            .write_all(body.as_bytes())
            .map_err(|e| format!("cannot hand gh the comment: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("cannot wait for gh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh could not post the comment: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let posted = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // Written after the comment is up, so a failure to post leaves them choosable.
    let mut task = task::load(&tasks::dir(ctx), id)?;
    for f in &picked {
        if !task.relayed.contains(&f.id) {
            task.relayed.push(f.id.clone());
        }
    }
    task.updated_at = crate::messaging::utc_stamp(crate::messaging::now_secs());
    task::save(&tasks::dir(ctx), &task)?;
    drop(lock);
    Ok(json!({ "relayed": chosen, "comment": posted }))
}

fn reviewers(ctx: &super::Context) -> Vec<String> {
    let configured: Vec<String> = ctx
        .resolved
        .config
        .as_ref()
        .and_then(|c| c.get("reviewBots"))
        .and_then(Value::as_array)
        .map(|bots| {
            bots.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if configured.is_empty() {
        DEFAULT_REVIEWERS.iter().map(|s| s.to_string()).collect()
    } else {
        configured
    }
}

/// The number at the end of a pull request URL.
fn pr_number(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("/pull/")?;
    let number = rest.split(['/', '?', '#']).next()?;
    (!number.is_empty() && number.chars().all(|c| c.is_ascii_digit())).then_some(number)
}

/// `gh api --jq` prints one object per line.
fn parse_findings(listed: &str, reviewers: &[String], relayed: &[String]) -> Vec<Finding> {
    listed
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|c| c.get("in_reply_to_id").is_none_or(Value::is_null))
        .filter(|c| {
            c.get("user")
                .and_then(Value::as_str)
                .is_some_and(|who| reviewers.iter().any(|r| r == who))
        })
        .filter_map(|c| {
            let id = match c.get("id")? {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                _ => return None,
            };
            let body = c.get("body").and_then(Value::as_str).unwrap_or_default();
            Some(Finding {
                relayed: relayed.contains(&id),
                id,
                author: c.get("user")?.as_str()?.to_string(),
                path: c
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                line: c
                    .get("line")
                    .and_then(Value::as_u64)
                    .or_else(|| c.get("original_line").and_then(Value::as_u64)),
                url: c
                    .get("html_url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                text: finding_text(body),
            })
        })
        .collect()
}

/// What of a review comment goes to Jules.
///
/// A review bot's comment is written for a person and folds away most of itself: hidden
/// markers, a committable suggestion, a prompt meant for an agent. The last is exactly what
/// Jules needs, so it is taken when it is there. Otherwise the comment goes without its hidden
/// and folded parts, which are the long ones.
fn finding_text(body: &str) -> String {
    if let Some(prompt) = agent_prompt(body) {
        return prompt;
    }
    let text = strip_folded(&strip_html_comments(body));
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

/// The inside of a `<details>` whose summary says it is a prompt for an agent, without its
/// code fence.
fn agent_prompt(body: &str) -> Option<String> {
    let at = body.find("Prompt for AI Agents")?;
    let rest = &body[at..];
    let rest = &rest[rest.find("</summary>")? + "</summary>".len()..];
    let inside = &rest[..rest.find("</details>")?];
    let text: Vec<&str> = inside
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect();
    let text = text.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn strip_html_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Drop every `<details>` block, nested ones with it.
fn strip_folded(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut rest = text;
    loop {
        let open = rest.find("<details");
        let close = rest.find("</details>");
        match (open, close) {
            (Some(o), c) if c.is_none_or(|c| o < c) => {
                if depth == 0 {
                    out.push_str(&rest[..o]);
                }
                depth += 1;
                rest = &rest[o + "<details".len()..];
            }
            (_, Some(c)) => {
                if depth == 0 {
                    out.push_str(&rest[..c]);
                }
                depth = depth.saturating_sub(1);
                rest = &rest[c + "</details>".len()..];
            }
            _ => {
                if depth == 0 {
                    out.push_str(rest);
                }
                return out;
            }
        }
    }
}

/// The comment Jules reads. In English, since that is what Jules is prompted in elsewhere,
/// with the person's own note first when there is one.
fn relay_body(picked: &[&Finding], note: Option<&str>) -> String {
    let mut out = String::from("Please address these review comments.\n");
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        out.push('\n');
        out.push_str(note);
        out.push('\n');
    }
    for (n, f) in picked.iter().enumerate() {
        let place = match f.line {
            Some(line) => format!("{}:{line}", f.path),
            None => f.path.clone(),
        };
        out.push_str(&format!("\n### {}. `{place}`\n\n{}\n", n + 1, f.text));
        if !f.url.is_empty() {
            out.push_str(&format!("\n({})\n", f.url));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_comment_is_read_down_to_its_prompt_for_an_agent() {
        let body = "_⚠️ Potential issue_\n\n**Guard the index.**\n\n<details>\n<summary>📝 Committable suggestion</summary>\n\n```diff\n-a\n+b\n```\n</details>\n\n<details>\n<summary>🤖 Prompt for AI Agents</summary>\n\n```\nIn src/a.rs around line 3, check the index before reading.\n```\n\n</details>\n\n<!-- fingerprinting:abc -->";
        assert_eq!(
            finding_text(body),
            "In src/a.rs around line 3, check the index before reading."
        );
    }

    #[test]
    fn a_review_comment_without_one_loses_its_hidden_and_folded_parts() {
        let body = "Use a constant here.\n\n\n<details>\n<summary>more</summary>\n<details>inner</details>\nlong\n</details>\n<!-- hidden -->\nThat is all.";
        assert_eq!(finding_text(body), "Use a constant here.\n\nThat is all.");
    }

    #[test]
    fn only_first_comments_by_a_review_bot_are_findings() {
        let listed = [
            r#"{"id":1,"path":"a.rs","line":3,"body":"x","html_url":"u1","in_reply_to_id":null,"user":"coderabbitai[bot]"}"#,
            r#"{"id":2,"path":"a.rs","line":3,"body":"reply","html_url":"u2","in_reply_to_id":1,"user":"coderabbitai[bot]"}"#,
            r#"{"id":3,"path":"b.rs","line":null,"original_line":9,"body":"y","html_url":"u3","user":"someone"}"#,
            r#"{"id":4,"path":"b.rs","line":null,"original_line":9,"body":"z","html_url":"u4","user":"coderabbitai[bot]"}"#,
        ]
        .join("\n");
        let found = parse_findings(
            &listed,
            &["coderabbitai[bot]".to_string()],
            &["4".to_string()],
        );
        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["1", "4"]);
        assert!(!found[0].relayed);
        assert!(found[1].relayed);
        assert_eq!(found[1].line, Some(9));
    }

    #[test]
    fn a_pr_number_is_read_from_its_url() {
        assert_eq!(pr_number("https://github.com/a/b/pull/12"), Some("12"));
        assert_eq!(
            pr_number("https://github.com/a/b/pull/12/files"),
            Some("12")
        );
        assert_eq!(pr_number("https://github.com/a/b/issues/12"), None);
        assert_eq!(pr_number("https://github.com/a/b/pull/x;y"), None);
    }

    #[test]
    fn the_relay_comment_lists_each_finding_with_its_place_and_the_note_first() {
        let f = Finding {
            id: "1".into(),
            author: "coderabbitai[bot]".into(),
            path: "src/a.rs".into(),
            line: Some(3),
            url: "https://github.com/a/b/pull/1#discussion_r1".into(),
            text: "Check the index.".into(),
            relayed: false,
        };
        let body = relay_body(&[&f], Some("Keep the public API as it is."));
        assert!(body.starts_with("Please address these review comments.\n\nKeep the public API"));
        assert!(body.contains("### 1. `src/a.rs:3`\n\nCheck the index."));
        assert!(body.contains("(https://github.com/a/b/pull/1#discussion_r1)"));
    }
}
