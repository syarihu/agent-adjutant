//! The read-through of a PR: what the reader hands in, checked, and what is kept of it.
//!
//! The JSON of these types is a contract with the board page and with the issues built on it
//! (the panel, the annotated diff, the pending review): see "The read-through" in
//! `docs/architecture.md`. The input types refuse unknown keys, so a reader's typo is an error it
//! can fix and not a field that is silently empty. The stored form keeps unknown keys (rule 10).

use serde::{Deserialize, Serialize};

use super::drafts::{MAX_DRAFT_CHARS, MAX_DRAFTS, MAX_DRAFTS_BYTES, MAX_PATH_CHARS, json_bytes};

/// The file a read-through is handed in as can be no bigger than this.
pub const MAX_READ_BYTES: usize = 2 * 1024 * 1024;
const MAX_POINTS: usize = 200;
const MAX_ORDER: usize = 1000;
const MAX_ANNOTATIONS: usize = 2000;
/// A line of `three`, in characters.
const MAX_THREE_CHARS: usize = 300;
/// The one version of the read-through there is.
pub const READ_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReadThrough {
    pub version: u32,
    /// The head SHA the reader read: 40 lower-case hex digits.
    pub sha: String,
    pub three: Three,
    /// What to look at, most dangerous first.
    #[serde(default)]
    pub points: Vec<Point>,
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
    #[serde(default)]
    pub drafts: Vec<Draft>,
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
pub enum Level {
    /// 要注意
    Attention,
    /// 確認
    Check,
    /// 参考
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Point {
    pub level: Level,
    pub path: String,
    /// Absent for the whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub title: String,
    pub why: String,
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
    /// A translation of a comment in the code.
    Tr,
    /// Background the comments do not give.
    Ex,
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
pub struct Draft {
    pub path: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub side: Side,
    pub body: String,
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
    /// The checked input with `id`, `madeAt` and `session` put in front of it.
    pub fn of(id: &str, made_at: &str, session: Option<&str>, read: &ReadThrough) -> Self {
        let rest = match serde_json::to_value(read) {
            Ok(serde_json::Value::Object(map)) => map,
            // A struct of this shape serialises to an object.
            _ => serde_json::Map::new(),
        };
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

    too_many(&mut problems, "points", read.points.len(), MAX_POINTS);
    // Most dangerous first is what the person reads down, so the order is part of the contract.
    let rank = |level: Level| match level {
        Level::Attention => 0,
        Level::Check => 1,
        Level::Info => 2,
    };
    if let Some(at) = read
        .points
        .windows(2)
        .position(|pair| rank(pair[1].level) < rank(pair[0].level))
    {
        problems.push(format!(
            "points[{}]: a point of a more dangerous level comes after a less dangerous one; \
             order points attention first, then check, then info, and move it up",
            at + 1
        ));
    }
    for (i, p) in read.points.iter().enumerate() {
        let at = format!("points[{i}]");
        path_problem(&mut problems, &at, &p.path);
        line_problem(&mut problems, &at, "line", p.line);
        required(&mut problems, &at, "title", &p.title);
        required(&mut problems, &at, "why", &p.why);
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
    }

    // The page seeds its drafts from these and saves them as the person's; what the drafts file
    // could never store, field by field or as a whole request, must not be accepted here, or the
    // person could never save them. The seeded form adds a key and a source to each draft, which
    // the headroom under the request limit (see `MAX_DRAFTS_BYTES`) is there for.
    if json_bytes(&read.drafts, &read.overall) > MAX_DRAFTS_BYTES {
        problems.push(format!(
            "drafts and overall together must be at most {MAX_DRAFTS_BYTES} bytes of JSON"
        ));
    }
    too_many(&mut problems, "drafts", read.drafts.len(), MAX_DRAFTS);
    for (i, d) in read.drafts.iter().enumerate() {
        let at = format!("drafts[{i}]");
        path_problem(&mut problems, &at, &d.path);
        if d.path.chars().count() > MAX_PATH_CHARS {
            problems.push(format!(
                "{at}: path must be at most {MAX_PATH_CHARS} characters"
            ));
        }
        if d.body.chars().count() > MAX_DRAFT_CHARS {
            problems.push(format!(
                "{at}: body must be at most {MAX_DRAFT_CHARS} characters"
            ));
        }
        line_problem(&mut problems, &at, "line", Some(d.line));
        line_problem(&mut problems, &at, "startLine", d.start_line);
        if d.start_line.is_some_and(|start| start > d.line) {
            problems.push(format!("{at}: startLine must not come after line"));
        }
        required(&mut problems, &at, "body", &d.body);
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
