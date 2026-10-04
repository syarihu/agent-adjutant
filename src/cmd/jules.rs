//! `adj jules` — hand a task's approved plan to Jules, and ask how that is going.
//!
//! The hub is the caller. It has a sub-agent write the plan in a worktree cut only to be read,
//! and once a person approves that plan it runs `adj jules start` with it, which starts the
//! session and writes its id onto the task record. From there the record is what
//! the board and the hub follow: the session id is the only thing about Jules kept locally.

use serde_json::{Value, json};

use crate::jules::{self, Chosen, findings, read_plan, relay};
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
    let ctx = crate::registry::context(args.repo, args.hub)?;
    let prompt = read_prompt(args.prompt)?;
    let session = jules::start(&ctx, args.id, &prompt, args.base)?;
    if args.json {
        println!("{}", json!({ "task": args.id, "session": session }));
        return Ok(());
    }
    println!("{} — handed to Jules as session {}", args.id, session.id);
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
    let ctx = crate::registry::context(args.repo, args.hub)?;
    let session_id = match (args.session, args.id) {
        (Some(session), _) => session.to_string(),
        (None, Some(id)) => task::get(&ctx.state, &ctx.repo.slug, id)?
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

/// `adj jules findings`: the review comments that could be passed on.
pub fn findings_cmd(
    repo: Option<&str>,
    hub: Option<&str>,
    id: &str,
    as_json: bool,
) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
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
    plan: Option<&str>,
    note: Option<&str>,
) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    let note = note.map(super::dash_is_stdin).transpose()?;
    let (chosen, note) = match plan {
        // A file, since it is prose the hub wrote and a note in it can hold any quote.
        Some(path) => {
            let path = crate::infra::paths::expand_home(path);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let plan: Value =
                serde_json::from_str(&text).map_err(|e| format!("bad relay plan: {e}"))?;
            let (chosen, planned) = read_plan(&plan)?;
            (chosen, note.or(planned))
        }
        None => (comments.iter().map(|c| Chosen::bare(c)).collect(), note),
    };
    let done = relay(&ctx, id, &chosen, note.as_deref())?;
    println!("passed {} comment(s) on to Jules", chosen.len());
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
            let path = crate::infra::paths::expand_home(path);
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
        ctx: &crate::registry::Context,
        key: &crate::infra::terminal::Hook,
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
                "working": jules::working(&found.state),
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

    fn ask(
        &self,
        ctx: &crate::registry::Context,
        key: &crate::infra::terminal::Hook,
        task_id: &str,
        session: &str,
    ) {
        let answer = jules::get(key, session);
        if let Ok(found) = &answer
            && let Err(e) = jules::follow(ctx, task_id, found)
        {
            eprintln!("adj serve: could not record the pull request of {task_id}: {e}");
        }
        if let Ok(found) = &answer
            && let Err(e) = jules::announce_review(ctx, task_id, found)
        {
            eprintln!("adj serve: could not bring up the review of {task_id}: {e}");
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = seen.get_mut(session) {
            entry.at = std::time::Instant::now();
            entry.asking = false;
            entry.answer = Some(answer);
        }
    }
}
