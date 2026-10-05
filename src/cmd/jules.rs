//! `adj jules` — hand a task's approved plan to Jules, and ask how that is going.
//!
//! The hub is the caller. It has a sub-agent write the plan in a worktree cut only to be read,
//! and once a person approves that plan it runs `adj jules start` with it, which starts the
//! session and writes its id onto the task record. From there the record is what
//! the board and the hub follow: the session id is the only thing about Jules kept locally.

use serde_json::{Value, json};

pub use crate::board::jobs::Watch;
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
