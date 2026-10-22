//! The read-through of a PR: what the reader hands in, checked, and what is kept of it. The
//! reader explains the change on its lines (`annotations`), and what it thinks deserves a question
//! or a change is a `concern` there, with the one draft review comment that goes with it.
//!
//! The JSON of these types is a contract with the board page and with the issues built on it
//! (the panel, the annotated diff, the pending review): see "The read-through" in
//! `docs/architecture.md`. The input types refuse unknown keys, so a reader's typo is an error it
//! can fix and not a field that is silently empty. The stored form keeps unknown keys (rule 10).

use serde::{Deserialize, Serialize};

use super::drafts::{MAX_DRAFT_CHARS, MAX_DRAFTS, MAX_DRAFTS_BYTES, MAX_PATH_CHARS, json_bytes};
use super::model::ReadMode;

/// The file a read-through is handed in as can be no bigger than this.
pub const MAX_READ_BYTES: usize = 2 * 1024 * 1024;
const MAX_ORDER: usize = 1000;
const MAX_ANNOTATIONS: usize = 2000;
/// A line of `three`, in characters.
const MAX_THREE_CHARS: usize = 300;
/// The version a reader hands in. A stored read-through of version 1 (with `points` and `drafts`)
/// is kept as it is on disk and is only read by the page, which tells the two apart by `version`.
pub const READ_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReadThrough {
    pub version: u32,
    /// The head SHA the reader read: 40 lower-case hex digits.
    pub sha: String,
    pub three: Three,
    /// The files in the order to read them.
    #[serde(default)]
    pub order: Vec<OrderEntry>,
    /// Files deleted whole.
    #[serde(default)]
    pub deleted: Vec<Deleted>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub tests: Tests,
    /// Questions for the author the body does not answer.
    #[serde(default)]
    pub unwritten: Vec<String>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    /// Text only: adj never submits a review, and never says approve or request changes.
    #[serde(default)]
    pub overall: Option<Overall>,
    /// Only for a PR already reviewed.
    #[serde(default)]
    pub since_review: Option<SinceReview>,
}

/// Purpose, approach and impact, one line each, and the one thing the PR turns on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Three {
    pub purpose: String,
    pub approach: String,
    pub impact: String,
    pub essence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Logic,
    Ui,
    Doc,
    Test,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OrderEntry {
    pub path: String,
    pub category: Category,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub skippable: bool,
}

/// A file deleted whole: the reason when the body or the commits give one, else a question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Deleted {
    pub path: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Decision {
    pub decision: String,
    pub gains: String,
    pub costs: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Tests {
    #[serde(default)]
    pub covered: Vec<String>,
    #[serde(default)]
    pub not_covered: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A translation of a comment in the code, written only when the comment is in another
    /// language than the person's.
    Tr,
    /// An explanation of the line: what it does and why, background the comments do not give,
    /// and what is risky about it stated as a fact.
    Ex,
    /// What the reader thinks deserves a question or a change, attached to its line. It comes
    /// with a draft review comment.
    Concern,
}

/// A note on one diff line, found by a substring of the line and not by its number, so it stays
/// on the right line when numbers shift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Annotation {
    pub path: String,
    pub anchor: String,
    /// Which match of `anchor` in that file, from 1. Absent is the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nth: Option<u32>,
    pub kind: Kind,
    pub text: String,
    /// The draft review comment a concern comes with, addressed to the author. One to one: a
    /// concern has exactly one draft, and nothing else has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Right,
    Left,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Overall {
    /// Becomes the body of the pending review. adj never chooses a verdict: that is the person's.
    pub body: String,
    /// The verdict the diff seems to call for, in words, one line. Shown to the person as text
    /// and never sent: kept apart from `body`, which is what gets posted. Optional, so a
    /// read-through made before it existed stays valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SinceReview {
    /// The commit of the person's last review.
    pub commit: String,
    #[serde(default)]
    pub comments: Vec<CommentStatus>,
    #[serde(default)]
    pub new_changes: Vec<NewChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// 対応済み
    Addressed,
    /// 返信だけ
    Replied,
    Untouched,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CommentStatus {
    /// GitHub's id of the review comment.
    pub comment_id: u64,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub status: Outcome,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NewChange {
    pub path: String,
    pub note: String,
}

/// A stored read-through: the checked input, with what the server adds. Keys this binary does
/// not know are kept (rule 10), so a newer binary's additions survive a read and a write back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stored {
    /// `owner/repo#N`.
    pub id: String,
    /// Stamp of when it was handed in.
    pub made_at: String,
    /// The reader's session, when it said one.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

impl Stored {
    /// The checked input with `id`, `madeAt` and `session` put in front of it, and `"mode":
    /// "split"` for a split proposal. A read-through carries no mode.
    pub fn of(
        id: &str,
        made_at: &str,
        session: Option<&str>,
        read: &ReadThrough,
        mode: ReadMode,
    ) -> Self {
        let mut rest = match serde_json::to_value(read) {
            Ok(serde_json::Value::Object(map)) => map,
            // A struct of this shape serialises to an object.
            _ => serde_json::Map::new(),
        };
        if let Some(mode) = mode.wire() {
            rest.insert("mode".to_string(), mode.into());
        }
        Stored {
            id: id.to_string(),
            made_at: made_at.to_string(),
            session: session.map(str::to_string),
            rest,
        }
    }
}

/// Every way `read` breaks the contract, one line each, so that a reader can fix them in one
/// pass. Empty means it is acceptable.
pub fn validate(read: &ReadThrough) -> Result<(), Vec<String>> {
    let mut problems: Vec<String> = Vec::new();
    if read.version != READ_VERSION {
        problems.push(format!("version must be {READ_VERSION}"));
    }
    if !is_sha(&read.sha) {
        problems.push("sha must be 40 lower-case hex digits (headRefOid of pr.json)".to_string());
    }
    for (name, line) in [
        ("purpose", &read.three.purpose),
        ("approach", &read.three.approach),
        ("impact", &read.three.impact),
        ("essence", &read.three.essence),
    ] {
        if blank(line) {
            problems.push(format!("three.{name} must not be blank"));
        } else if line.contains(['\n', '\r']) {
            problems.push(format!("three.{name} must be one line"));
        } else if line.chars().count() > MAX_THREE_CHARS {
            problems.push(format!(
                "three.{name} must be at most {MAX_THREE_CHARS} characters"
            ));
        }
    }

    too_many(&mut problems, "order", read.order.len(), MAX_ORDER);
    for (i, o) in read.order.iter().enumerate() {
        path_problem(&mut problems, &format!("order[{i}]"), &o.path);
    }

    for (i, d) in read.deleted.iter().enumerate() {
        let at = format!("deleted[{i}]");
        path_problem(&mut problems, &at, &d.path);
        // Blank is absent, here as everywhere: it must not stand in for a reason.
        let given = |v: &Option<String>| v.as_deref().is_some_and(|t| !blank(t));
        if given(&d.reason) == given(&d.question) {
            problems.push(format!(
                "{at}: exactly one of reason and question must be set (the other null)"
            ));
        }
    }

    for (i, d) in read.decisions.iter().enumerate() {
        let at = format!("decisions[{i}]");
        required(&mut problems, &at, "decision", &d.decision);
        required(&mut problems, &at, "gains", &d.gains);
        required(&mut problems, &at, "costs", &d.costs);
    }
    for (name, items) in [
        ("tests.covered", &read.tests.covered),
        ("tests.notCovered", &read.tests.not_covered),
        ("unwritten", &read.unwritten),
    ] {
        for (i, item) in items.iter().enumerate() {
            if blank(item) {
                problems.push(format!("{name}[{i}] must not be blank"));
            }
        }
    }

    too_many(
        &mut problems,
        "annotations",
        read.annotations.len(),
        MAX_ANNOTATIONS,
    );
    for (i, a) in read.annotations.iter().enumerate() {
        let at = format!("annotations[{i}]");
        path_problem(&mut problems, &at, &a.path);
        if blank(&a.anchor) {
            problems.push(format!("{at}: anchor must not be blank"));
        } else if a.anchor.contains(['\n', '\r']) {
            problems.push(format!("{at}: anchor must be a substring of one line"));
        }
        if a.nth == Some(0) {
            problems.push(format!("{at}: nth starts at 1"));
        }
        required(&mut problems, &at, "text", &a.text);
        match (a.kind, &a.draft) {
            (Kind::Concern, None) => {
                problems.push(format!("{at}: a concern must have a draft"));
            }
            (Kind::Concern, Some(draft)) => {
                if blank(draft) {
                    problems.push(format!("{at}: a concern must have a draft"));
                } else if draft.chars().count() > MAX_DRAFT_CHARS {
                    problems.push(format!(
                        "{at}: draft must be at most {MAX_DRAFT_CHARS} characters"
                    ));
                }
                // The path becomes the draft's path.
                if a.path.chars().count() > MAX_PATH_CHARS {
                    problems.push(format!(
                        "{at}: path must be at most {MAX_PATH_CHARS} characters"
                    ));
                }
            }
            (Kind::Tr | Kind::Ex, Some(_)) => {
                problems.push(format!("{at}: draft is only for a concern"));
            }
            (Kind::Tr | Kind::Ex, None) => {}
        }
    }

    // Each concern can become a comment when the person presses its button on the page, and is
    // saved as a draft; what the drafts file could never store, field by field or as a whole
    // request, must not be accepted here, or the person could never save them. The page adds a
    // key, a source and a sha to each draft, which the headroom under the request limit (see
    // `MAX_DRAFTS_BYTES`) is there for.
    let concerns: Vec<(&str, &str)> = read
        .annotations
        .iter()
        .filter(|a| a.kind == Kind::Concern)
        .map(|a| (a.path.as_str(), a.draft.as_deref().unwrap_or_default()))
        .collect();
    if concerns.len() > MAX_DRAFTS {
        problems.push(format!(
            "annotations has {} concerns, at most {MAX_DRAFTS} are kept",
            concerns.len()
        ));
    }
    if json_bytes(&concerns, &read.overall) > MAX_DRAFTS_BYTES {
        problems.push(format!(
            "the drafts of the concerns and overall together must be at most {MAX_DRAFTS_BYTES} bytes of JSON"
        ));
    }

    if let Some(overall) = &read.overall {
        required(&mut problems, "overall", "body", &overall.body);
        if overall.body.chars().count() > MAX_DRAFT_CHARS {
            problems.push(format!(
                "overall: body must be at most {MAX_DRAFT_CHARS} characters"
            ));
        }
        if let Some(suggestion) = &overall.suggestion {
            if blank(suggestion) {
                problems.push("overall: suggestion must not be blank (leave it out)".to_string());
            } else if suggestion
                .chars()
                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{85}'))
            {
                problems.push("overall: suggestion must be one line".to_string());
            } else if suggestion.chars().count() > MAX_THREE_CHARS {
                problems.push(format!(
                    "overall: suggestion must be at most {MAX_THREE_CHARS} characters"
                ));
            }
        }
    }

    if let Some(since) = &read.since_review {
        if blank(&since.commit) {
            problems.push("sinceReview.commit must not be blank".to_string());
        }
        for (i, c) in since.comments.iter().enumerate() {
            let at = format!("sinceReview.comments[{i}]");
            path_problem(&mut problems, &at, &c.path);
            line_problem(&mut problems, &at, "line", c.line);
            required(&mut problems, &at, "note", &c.note);
        }
        for (i, c) in since.new_changes.iter().enumerate() {
            let at = format!("sinceReview.newChanges[{i}]");
            path_problem(&mut problems, &at, &c.path);
            required(&mut problems, &at, "note", &c.note);
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// What a split proposal must be on top of a valid read-through: the proposal is `overall.body`
/// (the body of a pending review) and nothing else is taken from it. `three` is still required;
/// the lines of the diff were not read, so there is nothing to annotate (and so no concern to
/// draft), and a PR the person has not read has no earlier review to compare with. Empty means it
/// is acceptable.
pub fn validate_split(read: &ReadThrough) -> Result<(), Vec<String>> {
    let mut problems: Vec<String> = Vec::new();
    match &read.overall {
        Some(overall) if !blank(&overall.body) => {}
        _ => problems.push(
            "this is a split proposal: overall.body must hold the proposal, not blank".to_string(),
        ),
    }
    if !read.annotations.is_empty() {
        problems.push(
            "this is a split proposal: annotations must be [] (the diff's lines were not read)"
                .to_string(),
        );
    }
    if read.since_review.is_some() {
        problems.push("this is a split proposal: sinceReview must be null".to_string());
    }
    // Nothing was read line by line, so there is nothing to put in these.
    for (name, empty) in [
        ("order", read.order.is_empty()),
        ("deleted", read.deleted.is_empty()),
        ("decisions", read.decisions.is_empty()),
        ("unwritten", read.unwritten.is_empty()),
        (
            "tests",
            read.tests.covered.is_empty() && read.tests.not_covered.is_empty(),
        ),
    ] {
        if !empty {
            problems.push(format!(
                "this is a split proposal: {name} must be empty (the proposal is overall.body)"
            ));
        }
    }
    if read
        .overall
        .as_ref()
        .is_some_and(|o| o.suggestion.is_some())
    {
        problems.push(
            "this is a split proposal: overall.suggestion must be left out (no verdict is drawn from the structure)"
                .to_string(),
        );
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn blank(text: &str) -> bool {
    text.trim().is_empty()
}

pub(super) fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn too_many(problems: &mut Vec<String>, name: &str, len: usize, max: usize) {
    if len > max {
        problems.push(format!("{name} has {len} entries, at most {max} are kept"));
    }
}

fn required(problems: &mut Vec<String>, at: &str, name: &str, text: &str) {
    if blank(text) {
        problems.push(format!("{at}: {name} must not be blank"));
    }
}

/// A path in the repository: not blank, and not an absolute one.
fn path_problem(problems: &mut Vec<String>, at: &str, path: &str) {
    if blank(path) {
        problems.push(format!("{at}: path must not be blank"));
    } else if path.starts_with('/') {
        problems.push(format!("{at}: path must be relative to the repository"));
    }
}

fn line_problem(problems: &mut Vec<String>, at: &str, name: &str, line: Option<u32>) {
    if line == Some(0) {
        problems.push(format!("{at}: {name} starts at 1"));
    }
}
