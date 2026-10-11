//! A read that did not come to anything.

use std::path::Path;

use crate::infra::clock::utc_stamp;
use crate::task::PrRef;

use super::model::AiRead;
use super::store;

/// Mark the read of `pr` as `failed`, with `why`. Only a read that is `queued` or `running`
/// fails, and with `session_id` given only the one running under that session: a failure that
/// arrives after the read was asked for again, or after it was handed in, changes nothing.
/// `None` fails whichever session holds it. The `sha`, the counts and the read-through of an
/// earlier read stay, and so does the session, so a reader that was only slow can still hand in.
/// `true` when the read was failed by this call, `false` when it was not the one asked about.
pub fn fail_read(
    root: &Path,
    pr: &PrRef,
    session_id: Option<&str>,
    why: &str,
    now: i64,
) -> Result<bool, String> {
    let _lock = store::lock(root, &store::stem(pr))?;
    let mut record = store::load(root, pr)?.ok_or("no record of this pull request")?;
    let Some(ai) = record.ai.as_mut() else {
        return Ok(false);
    };
    let current = matches!(ai.status.as_str(), "queued" | "running")
        && session_id.is_none_or(|s| ai.session_id.as_deref() == Some(s));
    if !current {
        return Ok(false);
    }
    mark_failed(ai, why, now);
    store::save(root, &mut record)?;
    Ok(true)
}

/// Turn `ai` into a failed read with `why`: the one place that says what failing writes.
pub(super) fn mark_failed(ai: &mut AiRead, why: &str, now: i64) {
    ai.status = "failed".to_string();
    ai.error = Some(one_line(why));
    ai.finished_at = Some(utc_stamp(now));
}

/// How long the reason kept on a record may be, in characters.
const ERROR_CHARS: usize = 300;

/// `why` as one line of at most `ERROR_CHARS` characters: it comes from a reader (or a process's
/// output), and the page shows it in a row.
fn one_line(why: &str) -> String {
    let line = why.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= ERROR_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(ERROR_CHARS - 1).collect();
    cut.push('…');
    cut
}
