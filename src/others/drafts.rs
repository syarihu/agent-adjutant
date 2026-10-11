//! The person's draft review comments on a PR: what the page saves, checked, and what is kept.
//!
//! The JSON of these types is a contract with the board page and with the issue that posts the
//! drafts to GitHub: see "Draft comments" in `docs/architecture.md`. The input types refuse unknown
//! keys, so a page's typo is an error and not a field that is silently dropped. The stored form
//! keeps unknown keys (rule 10), so a newer binary may add keys beside these.

use serde::{Deserialize, Serialize};

use super::read_through::{Side, is_sha};

/// Drafts kept for one PR.
pub const MAX_DRAFTS: usize = 300;
/// A draft's body, and the overall comment, in characters.
pub const MAX_DRAFT_CHARS: usize = 20_000;
const MAX_KEY_CHARS: usize = 64;
/// The drafts and the overall comment together, as compact JSON. The page saves them in one
/// request, and the HTTP layer takes a body of at most 1 MiB; half of that is left for the keys the
/// page adds when it seeds a read-through's drafts, the id and the escaping of a body. The per-field
/// limits alone allow far more (300 bodies of 20000 characters).
pub const MAX_DRAFTS_BYTES: usize = 512 * 1024;
pub(super) const MAX_PATH_CHARS: usize = 1024;
/// The one version of the drafts there is.
pub const DRAFTS_VERSION: u32 = 1;

fn first_version() -> u32 {
    DRAFTS_VERSION
}

/// What the page sends: the drafts without the `id` and `savedAt` the server adds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DraftsInput {
    /// Absent is 1.
    #[serde(default = "first_version")]
    pub version: u32,
    /// The head of the read-through the reader's drafts were last taken from.
    #[serde(default)]
    pub from_sha: Option<String>,
    #[serde(default)]
    pub drafts: Vec<DraftItem>,
    #[serde(default)]
    pub overall: Option<OverallDraft>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Taken from the read-through.
    Reader,
    /// Typed by the person, or made from one of the reader's points.
    Person,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DraftItem {
    /// Names the draft among the others; stays the same while the text is edited.
    pub key: String,
    pub source: Source,
    /// Absent for a comment on the whole PR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Absent for a comment on the whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub side: Side,
    /// May be blank while the person is still writing it.
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OverallDraft {
    pub body: String,
}

/// Stored drafts: the checked input with what the server adds. Keys this binary does not know
/// are kept (rule 10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredDrafts {
    /// `owner/repo#N`.
    pub id: String,
    /// Stamp of the save.
    pub saved_at: String,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

impl StoredDrafts {
    /// The checked input with `id` and `savedAt` put in front of it.
    pub fn of(id: &str, saved_at: &str, input: &DraftsInput) -> Self {
        let rest = match serde_json::to_value(input) {
            Ok(serde_json::Value::Object(map)) => map,
            // A struct of this shape serialises to an object.
            _ => serde_json::Map::new(),
        };
        StoredDrafts {
            id: id.to_string(),
            saved_at: saved_at.to_string(),
            rest,
        }
    }
}

/// The size `MAX_DRAFTS_BYTES` is held to: `drafts` and `overall` serialised compactly, as
/// `JSON.stringify` writes them. A value that cannot be serialised counts as too big.
pub(super) fn json_bytes(drafts: &impl Serialize, overall: &impl Serialize) -> usize {
    serde_json::to_vec(&(drafts, overall)).map_or(usize::MAX, |v| v.len())
}

/// Every way `input` breaks the contract, one line each, so that the page can show them in one
/// pass. Empty means it is acceptable.
pub fn validate(input: &DraftsInput) -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();
    if input.version != DRAFTS_VERSION {
        problems.push(format!("version must be {DRAFTS_VERSION}"));
    }
    if let Some(sha) = &input.from_sha
        && !is_sha(sha)
    {
        problems.push("fromSha must be 40 lower-case hex digits, or null".to_string());
    }
    if input.drafts.len() > MAX_DRAFTS {
        problems.push(format!(
            "drafts has {} entries, at most {MAX_DRAFTS} are kept",
            input.drafts.len()
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    for (i, d) in input.drafts.iter().enumerate() {
        let at = format!("drafts[{i}]");
        if d.key.is_empty()
            || d.key.chars().count() > MAX_KEY_CHARS
            || !d
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            problems.push(format!(
                "{at}: key must be 1 to {MAX_KEY_CHARS} characters of letters, digits, _ and -"
            ));
        } else if seen.contains(&d.key.as_str()) {
            problems.push(format!("{at}: key {} is used by another draft", d.key));
        } else {
            seen.push(&d.key);
        }
        if let Some(path) = &d.path {
            if path.trim().is_empty() {
                problems.push(format!("{at}: path must not be blank (leave it out)"));
            } else if path.starts_with('/') {
                problems.push(format!("{at}: path must be relative to the repository"));
            } else if path.chars().count() > MAX_PATH_CHARS {
                problems.push(format!(
                    "{at}: path must be at most {MAX_PATH_CHARS} characters"
                ));
            }
        }
        if d.line.is_some() && d.path.is_none() {
            problems.push(format!("{at}: line needs a path"));
        }
        if d.start_line.is_some() && d.line.is_none() {
            problems.push(format!("{at}: startLine needs a line"));
        }
        for (name, value) in [("line", d.line), ("startLine", d.start_line)] {
            if value == Some(0) {
                problems.push(format!("{at}: {name} starts at 1"));
            }
        }
        if let (Some(start), Some(line)) = (d.start_line, d.line)
            && start > line
        {
            problems.push(format!("{at}: startLine must not come after line"));
        }
        if d.body.chars().count() > MAX_DRAFT_CHARS {
            problems.push(format!(
                "{at}: body must be at most {MAX_DRAFT_CHARS} characters"
            ));
        }
    }
    if let Some(overall) = &input.overall
        && overall.body.chars().count() > MAX_DRAFT_CHARS
    {
        problems.push(format!(
            "overall: body must be at most {MAX_DRAFT_CHARS} characters"
        ));
    }
    if json_bytes(&input.drafts, &input.overall) > MAX_DRAFTS_BYTES {
        problems.push(format!(
            "the drafts and the overall comment together must be at most {MAX_DRAFTS_BYTES} bytes of JSON"
        ));
    }
    problems
}
