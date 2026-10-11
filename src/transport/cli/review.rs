//! `adj review`: ask for a read-through of a PR, collect what a reader reads, hand one in.
//!
//! None of these takes a `Context` from git: they run from a reader's scratch directory, which
//! is no checkout, and read the state directory the tab was given.

use std::path::Path;

use super::args::{
    ReviewContextArgs, ReviewFailArgs, ReviewRequestArgs, ReviewRunArgs, ReviewSayArgs,
    ReviewSubmitArgs,
};
use crate::board::review::{self, RequestRefused};
use crate::infra::env::REVIEW_SESSION_ENV;
use crate::lifecycle;
use crate::others::{self, ReadMode, Requested, SubmitError};
use crate::registry;
use crate::task::PrRef;
use crate::transport::wording::{queue_warning, review_refusal};

fn pr_named(text: &str) -> Result<PrRef, String> {
    others::pr_of(text)
        .ok_or_else(|| format!("not a pull request: {text} (a pull request URL or owner/repo#N)"))
}

pub fn review_request(args: &ReviewRequestArgs) -> Result<(), String> {
    let root = registry::state_root(None);
    pr_named(&args.pr)?;
    let mode = if args.split {
        ReadMode::Split
    } else {
        ReadMode::Read
    };
    let record = match review::request(&root, &args.pr, mode) {
        Ok(asked) => {
            if let Some(why) = &asked.queue_error {
                eprintln!("adjutant: {}", queue_warning(why));
            }
            match asked.requested {
                Requested::Queued(record) => record,
                Requested::Already(record) => {
                    println!("{} is already being read or waiting for a place", record.id);
                    record
                }
            }
        }
        Err(RequestRefused::BadId) => return Err(format!("not a pull request: {}", args.pr)),
        Err(RequestRefused::Refused(why)) => return Err(review_refusal(&why)),
    };
    let status = record.ai.as_ref().map(|ai| ai.status.as_str());
    println!(
        "{}: {}",
        record.id,
        match status {
            Some("running") => "being read",
            Some("failed") => "failed",
            _ => "waiting for a place",
        }
    );
    Ok(())
}

pub fn review_context(args: &ReviewContextArgs) -> Result<(), String> {
    let pr = pr_named(&args.pr)?;
    let root = registry::state_root(None);
    let out = Path::new(args.out.as_deref().unwrap_or("."));
    let session = std::env::var(REVIEW_SESSION_ENV)
        .ok()
        .filter(|session| !session.trim().is_empty());
    let summary = others::read_context(&root, &pr, out, session.as_deref())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
    );
    Ok(())
}

pub fn review_submit(args: &ReviewSubmitArgs) -> Result<(), String> {
    let pr = pr_named(&args.pr)?;
    let root = registry::state_root(None);
    let text = std::fs::read_to_string(&args.file)
        .map_err(|e| format!("cannot read {}: {e}", args.file))?;
    let session = std::env::var(REVIEW_SESSION_ENV)
        .ok()
        .filter(|session| !session.trim().is_empty());
    match lifecycle::review::submit(&root, &pr, &text, session.as_deref()) {
        Ok(done) => {
            if let Some(why) = &done.queue_error {
                eprintln!("adjutant: {}", queue_warning(why));
            }
            println!("handed in: {}", done.submitted.record.id);
            Ok(())
        }
        Err(SubmitError::Invalid(problems)) => {
            for problem in &problems {
                eprintln!("{problem}");
            }
            Err(format!(
                "the read-through was not taken: {} problem(s); fix the file and hand it in again",
                problems.len()
            ))
        }
        Err(SubmitError::NoRecord) => Err("no such pull request on the list".to_string()),
        Err(SubmitError::NotAsked) => {
            Err("nobody asked for a read-through of this pull request".to_string())
        }
        Err(SubmitError::Superseded) => Err(
            "this reader was superseded: the read was asked for again, and a newer reader hands it in; nothing was written".to_string(),
        ),
        Err(SubmitError::Failed(why)) => Err(why),
    }
}

/// A reader's word to the code-review hub, and to nobody else: the address is fixed, and no
/// option can change it, wake someone or say who is speaking. The reader's session and PR go in
/// what is sent, from the environment and the arguments and not from anything it can claim.
pub fn review_say(args: &ReviewSayArgs) -> Result<(), String> {
    let pr = pr_named(&args.pr)?;
    let root = registry::state_root(None);
    let ctx = registry::review_context(&root)?;
    let session = std::env::var(REVIEW_SESSION_ENV)
        .ok()
        .filter(|session| !session.trim().is_empty());
    // Only a reader speaks here: a person has `adj send`. The reader is the one the record is
    // waiting for, and no other may speak for this PR.
    let Some(session) = session.as_deref() else {
        return Err(
            "only a reader can say something this way (ADJUTANT_REVIEW_SESSION is not set); to write to the hub yourself use adj send --hub @review".to_string(),
        );
    };
    {
        let id = others::pr_id(&pr);
        let current = others::list(&root)
            .into_iter()
            .find(|r| r.id == id)
            .and_then(|r| r.ai)
            .and_then(|ai| ai.session_id);
        if current.as_deref() != Some(session) {
            return Err(
                "this reader is not the one running this read (it was asked for again, or it is over); nothing was written".to_string(),
            );
        }
    }
    let body = super::stdin::dash_is_stdin(&args.body)?;
    let message = crate::mail::Message {
        from: format!("adjutant-read-{}", others::reader_stem(&pr)),
        worktree: None,
        kind: args.kind.clone(),
        subject: args.subject.trim().to_string(),
        body: format!(
            "Pull request: {}\nReader session: {}\n\n{}",
            others::pr_id(&pr),
            session,
            body
        ),
    };
    let outcome = crate::mail::deliver_to_hub_with_wake(&ctx, &message, true, None)?;
    println!("told {}: {}", ctx.repo.hub_name, outcome.path.display());
    Ok(())
}

pub fn review_fail(args: &ReviewFailArgs) -> Result<(), String> {
    let pr = pr_named(&args.pr)?;
    let root = registry::state_root(None);
    let why = args.why.trim();
    if why.is_empty() {
        return Err("say why the read could not be made".to_string());
    }
    let session = std::env::var(REVIEW_SESSION_ENV)
        .ok()
        .filter(|session| !session.trim().is_empty());
    match lifecycle::review::fail(&root, &pr, session.as_deref(), why) {
        Ok(queue_error) => {
            if let Some(why) = queue_error {
                eprintln!("adjutant: {}", queue_warning(&why));
            }
            println!("failed: {}", args.pr);
            Ok(())
        }
        Err(lifecycle::review::FailError::NotCurrent) => Err(
            "this reader is not the one running this read (it was asked for again, or it is over); nothing was written".to_string(),
        ),
        Err(lifecycle::review::FailError::Failed(why)) => Err(why),
    }
}

/// What a reader's window runs. Returns only when the agent could not be started.
pub fn review_run(args: &ReviewRunArgs) -> Result<(), String> {
    let pr = pr_named(&args.pr)?;
    let root = registry::state_root(None);
    Err(lifecycle::review::run_reader(&root, &pr, &args.session))
}
