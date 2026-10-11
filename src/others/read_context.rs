//! The material a reader reads, written next to it: the one place a reader's GitHub reads happen.
//!
//! Every call is a read-only request, never a mutation: no `-X`, `-f`, `-F` or `--input` is built
//! here (`gh pr view` is a GraphQL query, and a query only reads), so what the reader can learn is
//! bounded by what this function asks for, and it has no way to write.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::task::PrRef;

use super::github::{Gh, answer, viewer};
use super::model::ReadMode;
use super::reader_dir::reader_dir;
use super::store;

/// How long collecting the material may take, all of it.
const CONTEXT_TIMEOUT: Duration = Duration::from_secs(60);

/// The files this writes only sometimes. They are removed before anything is written, so a
/// directory used for an earlier read (an earlier head, a review you have since made) cannot
/// leave one behind to be read as this PR's.
const OPTIONAL_FILES: [&str; 4] = [
    "diff.patch",
    "hunks.patch",
    "my-last-review.json",
    "since-review.patch",
];

/// The files a split proposal does not read, which a read-through writes every time: removed for
/// the same reason when the read is a split, so an earlier read's comments are not taken for it.
const NOT_FOR_A_SPLIT: [&str; 2] = ["review-comments.json", "reviews.json"];

/// How many files or commits `gh pr view` returns at most; a list this long may be cut.
const VIEW_LIMIT: usize = 100;

/// What `gh pr view` is asked for: the body, the commit messages and the files.
const VIEW_FIELDS: &str =
    "number,title,body,author,baseRefName,headRefName,headRefOid,additions,deletions,files,commits";

/// What was collected, for the reader to start from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSummary {
    /// The head SHA the material is of.
    pub head_sha: String,
    pub files: Vec<ContextFile>,
    /// The files written, by name, in `out`.
    pub written: Vec<String>,
    /// Whether this is a PR the person has reviewed and that has moved since.
    pub rereview: bool,
    /// `read` for a read-through, `split` for a split proposal: the record's, not the reader's.
    pub mode: &'static str,
    /// What could not be collected and was left out, which is not a failure.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// Collect the material of `pr` into `out` with the real `gh`.
///
/// `session` is the reader's (`ADJUTANT_REVIEW_SESSION`): when given it must be the one the record
/// names, so a superseded reader fetches nothing. Only a person by hand says none.
pub fn read_context(
    root: &Path,
    pr: &PrRef,
    out: &Path,
    session: Option<&str>,
) -> Result<ContextSummary, String> {
    read_context_with(root, pr, out, session, &|args, deadline| {
        crate::infra::gh::run(None, args, deadline)
    })
}

/// `read_context`, with the way to run `gh` handed in.
pub(crate) fn read_context_with(
    root: &Path,
    pr: &PrRef,
    out: &Path,
    session: Option<&str>,
    gh: Gh,
) -> Result<ContextSummary, String> {
    // Before anything is read or written: `out` is a place the reader names, and the reader is
    // the one the record is waiting for.
    let out = own_dir(root, pr, out)?;
    if pr.host != "github.com" {
        return Err(format!("only github.com is read, not {}", pr.host));
    }
    let record = store::load(root, pr)?.ok_or("no record of this pull request")?;
    if let Some(session) = session
        && record.ai.as_ref().and_then(|ai| ai.session_id.as_deref()) != Some(session)
    {
        return Err(
            "this reader was superseded: the read was asked for again, or it is over; nothing was fetched or written"
                .to_string(),
        );
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let mode = ReadMode::of(record.ai.as_ref());
    for name in OPTIONAL_FILES {
        crate::infra::fs::remove_if_present(&out.join(name))?;
    }
    if mode == ReadMode::Split {
        for name in NOT_FOR_A_SPLIT {
            crate::infra::fs::remove_if_present(&out.join(name))?;
        }
    }

    let deadline = Instant::now() + CONTEXT_TIMEOUT;
    let url = format!(
        "https://{}/{}/{}/pull/{}",
        pr.host, pr.owner, pr.repo, pr.number
    );
    let api = format!("repos/{}/{}/pulls/{}", pr.owner, pr.repo, pr.number);
    let mut written: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    let view = get(gh, &["pr", "view", &url, "--json", VIEW_FIELDS], deadline)?;
    let mut view: Value =
        serde_json::from_str(&view).map_err(|e| format!("cannot read gh's pull request: {e}"))?;
    let head = view
        .get("headRefOid")
        .and_then(Value::as_str)
        .filter(|sha| is_hex(sha, 40..=40))
        .ok_or("gh did not say the head commit")?
        .to_string();
    let files = files_of(&view);
    let commits = view
        .get("commits")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if files.len() >= VIEW_LIMIT || (files.len() as u64) < record.changed_files {
        notes.push(format!(
            "the file list may be cut: {} of {} files were returned; say in `unwritten` what you could not see, and do not guess",
            files.len(),
            record.changed_files
        ));
    }
    if commits >= VIEW_LIMIT {
        notes.push(format!(
            "the commit list may be cut at {commits}; say in `unwritten` that some commit messages were not read, and do not guess their reasons"
        ));
    }
    view["myReview"] = serde_json::to_value(&record.my_review).map_err(|e| e.to_string())?;
    put_json(&out, "pr.json", &view, &mut written)?;

    if mode == ReadMode::Split {
        // The proposal is drafted from the structure alone: the diff's lines, the comments and
        // the earlier review are not fetched, and the files are not written.
        split_material(gh, &url, &api, &out, deadline, &mut written, &mut notes)?;
        return Ok(ContextSummary {
            head_sha: head,
            files,
            written,
            mode: "split",
            rereview: false,
            notes,
        });
    }

    // A diff that cannot be fetched (too big for GitHub to give) is not the end of the read: the
    // reader still has the files and the messages, and says what it could not see.
    match get(gh, &["pr", "diff", &url], deadline) {
        Ok(diff) => put_text(&out, "diff.patch", &diff, &mut written)?,
        Err(why) => notes.push(format!(
            "diff.patch could not be fetched ({why}); put what you could not read in `points` or `unwritten`, and do not guess"
        )),
    }

    let comments = paginated(gh, &format!("{api}/comments"), deadline)?;
    put_json(&out, "review-comments.json", &comments, &mut written)?;
    let reviews = paginated(gh, &format!("{api}/reviews"), deadline)?;
    put_json(&out, "reviews.json", &reviews, &mut written)?;

    let mut rereview = false;
    if let Some(mine) = record.my_review.as_ref().filter(|m| m.commit != head) {
        rereview = true;
        let thread =
            my_last_review(gh, deadline, mine, &reviews, &comments).unwrap_or_else(|why| {
                notes.push(format!("my-last-review.json is empty: {why}"));
                Value::Array(Vec::new())
            });
        put_json(&out, "my-last-review.json", &thread, &mut written)?;
        // A commit that is not hex must not become part of a path.
        if is_hex(&mine.commit, 7..=40) {
            let compare = format!(
                "repos/{}/{}/compare/{}...{head}",
                pr.owner, pr.repo, mine.commit
            );
            let diff = get(
                gh,
                &["api", "-H", "Accept: application/vnd.github.diff", &compare],
                deadline,
            );
            match diff {
                Ok(text) => put_text(&out, "since-review.patch", &text, &mut written)?,
                // After a force-push the reviewed commit may be gone.
                Err(why) => notes.push(format!("since-review.patch not written: {why}")),
            }
        } else {
            notes.push("since-review.patch not written: the reviewed commit is not a SHA".into());
        }
    }

    Ok(ContextSummary {
        head_sha: head,
        files,
        written,
        mode: "read",
        rereview,
        notes,
    })
}

/// Write `hunks.patch`: the file headers and the `@@` lines of the PR's diff, and none of its
/// lines. GitHub refuses to give a diff this large in one piece (it is what puts a PR out of
/// review), so when `gh pr diff` fails the headers are taken from the patches of the files list.
fn split_material(
    gh: Gh,
    url: &str,
    api: &str,
    out: &Path,
    deadline: Instant,
    written: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let diff_error = match get(gh, &["pr", "diff", url], deadline) {
        Ok(diff) => return put_text(out, "hunks.patch", &hunk_headers(&diff), written),
        Err(why) => why,
    };
    match paginated(gh, &format!("{api}/files"), deadline) {
        Ok(files) => {
            let (text, missing) = headers_from_files(&files);
            for path in missing {
                notes.push(format!("hunk headers of {path} are not available"));
            }
            notes.push(format!(
                "hunks.patch was made from the files list, not the diff ({diff_error})"
            ));
            put_text(out, "hunks.patch", &text, written)
        }
        Err(why) => {
            notes.push(format!(
                "hunks.patch could not be made: the diff ({diff_error}) and the files list ({why}) were both refused; draft the proposal from the file list in pr.json alone, and say so in the proposal"
            ));
            Ok(())
        }
    }
}

/// What the `files` endpoint gives as hunks.patch lines, and the paths whose `patch` it left out
/// (GitHub omits it for a file that is too large or binary).
fn headers_from_files(files: &Value) -> (String, Vec<String>) {
    let mut text = String::new();
    let mut missing = Vec::new();
    for file in files.as_array().into_iter().flatten() {
        let Some(path) = file.get("filename").and_then(Value::as_str) else {
            continue;
        };
        let number = |key: &str| file.get(key).and_then(Value::as_u64).unwrap_or(0);
        let status = file
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("changed");
        // A renamed file names where it came from, as a diff does.
        let old = file
            .get("previous_filename")
            .and_then(Value::as_str)
            .filter(|old| *old != path);
        text.push_str(&format!("diff --git a/{} b/{path}\n", old.unwrap_or(path)));
        if let Some(old) = old {
            text.push_str(&format!("rename from {old}\nrename to {path}\n"));
        }
        text.push_str(&format!(
            "# status: {status} +{} -{}\n",
            number("additions"),
            number("deletions")
        ));
        match file.get("patch").and_then(Value::as_str) {
            Some(patch) => text.push_str(&hunk_headers(patch)),
            // A pure rename has no hunk to be missing.
            None if number("additions") + number("deletions") > 0 => missing.push(path.to_string()),
            None => {}
        }
    }
    (text, missing)
}

/// The lines of a unified diff that say which file and which lines, without the lines.
///
/// A hunk is read by its counts (`@@ -a,b +c,d @@`, a missing count is 1), not by what a line
/// looks like: a deleted line whose text starts `-- ` shows as `--- ...`, and an added line may
/// start `diff --git`, and neither is a header.
pub(crate) fn hunk_headers(diff: &str) -> String {
    const KEPT: [&str; 13] = [
        "diff --git ",
        "old mode ",
        "new mode ",
        "new file mode ",
        "deleted file mode ",
        "similarity index ",
        "dissimilarity index ",
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
        "index ",
        "Binary files ",
    ];
    let mut out = String::new();
    // Lines of the current hunk still to come: old side, new side.
    let (mut old, mut new) = (0u64, 0u64);
    for line in diff.lines() {
        if old > 0 || new > 0 {
            match line.as_bytes().first() {
                Some(b'-') => old = old.saturating_sub(1),
                Some(b'+') => new = new.saturating_sub(1),
                // A context line; one with its trailing space stripped is empty.
                Some(b' ') | None => {
                    old = old.saturating_sub(1);
                    new = new.saturating_sub(1);
                }
                // `\ No newline at end of file` is no line of either side.
                Some(_) => {}
            }
            continue;
        }
        if let Some((o, n)) = hunk_counts(line) {
            (old, new) = (o, n);
            out.push_str(line);
            out.push('\n');
        } else if line.starts_with("--- ")
            || line.starts_with("+++ ")
            || KEPT.iter().any(|prefix| line.starts_with(prefix))
        {
            out.push_str(line);
            out.push('\n');
        }
    }
    if old > 0 || new > 0 {
        out.push_str("# note: the diff was cut inside a hunk; hunks.patch may be incomplete\n");
    }
    out
}

/// The old and new line counts of a `@@ -a,b +c,d @@` line; `None` for any other line.
fn hunk_counts(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let count = |range: &str| -> Option<u64> {
        match range.split_once(',') {
            Some((_, n)) => n.parse().ok(),
            None => range.parse::<u64>().ok().map(|_| 1),
        }
    };
    Some((count(old)?, count(new)?))
}

/// `out` made absolute, when it is the reader directory of this very PR, `<state>/review/<stem>/`
/// and no other: a reader names where to write, and it must not be able to overwrite another
/// reader's material. Compared after symlinks are followed on the part that exists, so a state
/// directory under a link and a working directory that names the real path agree.
fn own_dir(root: &Path, pr: &PrRef, out: &Path) -> Result<PathBuf, String> {
    let own = reader_dir(root, pr);
    let refuse = || {
        format!(
            "{} is not the directory of this pull request's reader ({}): the material is written there and nowhere else",
            out.display(),
            own.display()
        )
    };
    let absolute = std::path::absolute(out).map_err(|e| e.to_string())?;
    // `..` would walk out after the check.
    if absolute.components().any(|c| c == Component::ParentDir) {
        return Err(refuse());
    }
    let wanted = resolved(&std::path::absolute(&own).map_err(|e| e.to_string())?);
    let target = resolved(&absolute);
    if target == wanted {
        Ok(target)
    } else {
        Err(refuse())
    }
}

/// `path` with symlinks followed as far as it exists, and the rest as it was written.
fn resolved(path: &Path) -> PathBuf {
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    let mut here = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(here) {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (here.parent(), here.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                here = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn get(gh: Gh, args: &[&str], deadline: Instant) -> Result<String, String> {
    answer(gh(args, deadline)?)
}

/// All pages of a list endpoint, as one array. `gh api --paginate` prints one array per page,
/// back to back.
fn paginated(gh: Gh, endpoint: &str, deadline: Instant) -> Result<Value, String> {
    let text = get(gh, &["api", "--paginate", endpoint], deadline)?;
    let mut all: Vec<Value> = Vec::new();
    for page in serde_json::Deserializer::from_str(&text).into_iter::<Value>() {
        match page.map_err(|e| format!("cannot read gh's answer for {endpoint}: {e}"))? {
            Value::Array(items) => all.extend(items),
            other => return Err(format!("gh's answer for {endpoint} is not a list: {other}")),
        }
    }
    Ok(Value::Array(all))
}

fn files_of(view: &Value) -> Vec<ContextFile> {
    view.get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|f| {
                    Some(ContextFile {
                        path: f.get("path")?.as_str()?.to_string(),
                        additions: f.get("additions").and_then(Value::as_u64).unwrap_or(0),
                        deletions: f.get("deletions").and_then(Value::as_u64).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The comments of the person's last review, each with the replies to it. The review is found
/// among `reviews` as the viewer's latest submitted one on the commit `mine` names; one that
/// cannot be found is an error here and a note to the caller, never a guess.
fn my_last_review(
    gh: Gh,
    deadline: Instant,
    mine: &super::model::MyReview,
    reviews: &Value,
    comments: &Value,
) -> Result<Value, String> {
    let me = viewer(gh, deadline)?;
    let review_id = reviews
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            r.pointer("/user/login")
                .and_then(Value::as_str)
                .is_some_and(|login| login.eq_ignore_ascii_case(&me))
                && r.get("commit_id").and_then(Value::as_str) == Some(mine.commit.as_str())
                // A pending review is not submitted, and GitHub spells its state in capitals.
                && !r
                    .get("state")
                    .and_then(Value::as_str)
                    .is_some_and(|state| state.eq_ignore_ascii_case("pending"))
        })
        .filter_map(|r| {
            Some((
                r.get("submitted_at")?.as_str()?.to_string(),
                r.get("id")?.as_u64()?,
            ))
        })
        .max()
        .map(|(_, id)| id)
        .ok_or("your review on that commit is not among the reviews")?;
    let all = comments.as_array().cloned().unwrap_or_default();
    let id_of = |c: &Value| c.get("id").and_then(Value::as_u64);
    let mut ids: Vec<u64> = all
        .iter()
        .filter(|c| c.get("pull_request_review_id").and_then(Value::as_u64) == Some(review_id))
        .filter_map(id_of)
        .collect();
    // Replies to those, and replies to replies, until no more come in.
    loop {
        let before = ids.len();
        for c in &all {
            let parent = c.get("in_reply_to_id").and_then(Value::as_u64);
            if let (Some(own), Some(parent)) = (id_of(c), parent)
                && ids.contains(&parent)
                && !ids.contains(&own)
            {
                ids.push(own);
            }
        }
        if ids.len() == before {
            break;
        }
    }
    Ok(Value::Array(
        all.into_iter()
            .filter(|c| id_of(c).is_some_and(|id| ids.contains(&id)))
            .collect(),
    ))
}

fn is_hex(text: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn put_json(
    out: &Path,
    name: &str,
    value: &Value,
    written: &mut Vec<String>,
) -> Result<(), String> {
    crate::infra::fs::write_json(&out.join(name), value)?;
    written.push(name.to_string());
    Ok(())
}

fn put_text(out: &Path, name: &str, text: &str, written: &mut Vec<String>) -> Result<(), String> {
    store::write_text(&out.join(name), text)?;
    written.push(name.to_string());
    Ok(())
}
