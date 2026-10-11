use std::os::unix::process::CommandExt;
use std::path::Path;

use crate::infra::clock::now_secs;
use crate::infra::env::REVIEW_SESSION_ENV;
use crate::infra::terminal;
use crate::kernel::runner;
use crate::lifecycle::{agent_command, agent_hooks_for};
use crate::others::{self, ReadMode};
use crate::registry::{self, agent_env};
use crate::task::PrRef;

/// Become the reader of `pr` that runs as `session`: write down this process and its window,
/// then replace this process with the agent. Returns only when it cannot, with what to tell the
/// person.
///
/// Refused when the record no longer names this session (the read was asked for again, or it
/// was failed), so a superseded launch starts no agent.
pub fn run_reader(root: &Path, pr: &PrRef, session: &str) -> String {
    let ctx = match registry::review_context(root) {
        Ok(ctx) => ctx,
        Err(e) => return e,
    };
    let id = others::pr_id(pr);
    let stem = others::reader_stem(pr);
    let pid = std::process::id();
    if let Err(e) = others::note_reader(
        root,
        pr,
        session,
        pid,
        registry::ps_started(pid),
        Some(terminal::own_location(&ctx.settings.terminal)),
    ) {
        return e;
    }
    // The reader works in its own directory, whatever this was started from.
    let scratch = others::reader_dir(root, pr);
    if let Err(e) =
        std::fs::create_dir_all(&scratch).and_then(|()| std::env::set_current_dir(&scratch))
    {
        let why = format!("cannot work in {}: {e}", scratch.display());
        let _ = others::fail_read(root, pr, Some(session), &why, now_secs());
        return why;
    }
    let mut env = agent_env(&ctx);
    env.push((REVIEW_SESSION_ENV.to_string(), session.to_string()));
    let template = ctx.settings.reviewer_runner.as_deref();
    let hooks = agent_hooks_for(
        &ctx.state,
        template.unwrap_or(runner::DEFAULT_REVIEWER_RUNNER),
    );
    let settings = hooks.path().map(|path| path.to_string_lossy());
    // The mode is the record's, as the request left it: what asked is not the reader's to say.
    let mode = match others::record(root, pr) {
        Ok(record) => ReadMode::of(record.as_ref().and_then(|r| r.ai.as_ref())),
        Err(why) => {
            let _ = others::fail_read(root, pr, Some(session), &why, now_secs());
            return why;
        }
    };
    let prompt = reader_prompt(&id, mode);
    let command = runner::reviewer_command(
        template,
        &env,
        &format!("adjutant-read-{stem}"),
        session,
        &prompt,
        settings.as_deref(),
    );
    let error = agent_command(&command).exec();
    let why = format!("cannot start the reader: {error}");
    let _ = others::fail_read(root, pr, Some(session), &why, now_secs());
    why
}

/// What the reader's agent is told to do: the procedure is fetched, and the mode picks the part
/// of it that applies.
fn reader_prompt(id: &str, mode: ReadMode) -> String {
    match mode {
        ReadMode::Read => format!(
            "Fetch the adj-reviewer procedure with `adj skill adj-reviewer` (no MCP tools are available to you), and read the pull request {id} as it says."
        ),
        ReadMode::Split => format!(
            "Fetch the adj-reviewer procedure with `adj skill adj-reviewer` (no MCP tools are available to you), and draft a split proposal for the pull request {id} as its section \"Split proposal\" says."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_the_split_section_only_for_a_split() {
        let read = reader_prompt("o/r#1", ReadMode::Read);
        assert!(read.contains("read the pull request o/r#1 as it says"));
        assert!(!read.contains("Split proposal"));
        let split = reader_prompt("o/r#1", ReadMode::Split);
        assert!(split.contains("draft a split proposal for the pull request o/r#1"));
        assert!(split.contains("\"Split proposal\""));
    }
}
