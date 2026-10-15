use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What the aide has put in the display area of its pane, and what `GET /api/aide/display`
/// answers: the parts of the topic being talked about, newest first, and the topics before it.
///
/// The page draws this shape as it is (`aideDisplayOf` in `src/ui/aide-display.js`). `seq` numbers
/// the parts and is not drawn. A key this binary does not know is kept (rule 10).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AideDisplay {
    /// `None` until something has been shown.
    pub topic: Option<Topic>,
    /// Every part of `topic`, newest first.
    pub parts: Vec<AidePart>,
    /// The topics before it, newest first.
    pub earlier: Vec<Topic>,
    /// The next part's number.
    pub seq: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What a run of parts is about. The key says whether a part still belongs; the label is what
/// the person reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Topic {
    pub key: String,
    pub label: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Topic {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Topic {
            key: key.into(),
            label: label.into(),
            extra: Map::new(),
        }
    }
}

/// One thing the aide showed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AidePart {
    pub id: String,
    /// `gate`, `waits`, `reviews`, `read`, `task`, `pr`, `session`, `request` or `diff`; the page draws a kind it has no
    /// renderer for as one it cannot show yet.
    pub kind: String,
    /// What it is about, by reference. `None` for a part that is about nothing in particular,
    /// such as the list of waits.
    #[serde(rename = "ref")]
    pub reference: Option<PartRef>,
    /// The key of the topic it was shown under.
    pub topic: String,
    /// When it was shown, in epoch seconds.
    pub shown_at: i64,
    /// Who showed it.
    pub by: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What a part is about, by reference only: the page reads the content from the records.
///
/// Which fields a kind uses: `gate`, `session` and `request` `board` + `id` (a request's is its task); `task` `board` + `id`, or
/// `board` + `parent`; `pr` `board` + `id` or `pr`; `read` `pr`, with `point` or `level`; `diff`
/// `path`, `from`, `to`, `side`, and a source (`pr`, or `board` + `id` of a gate). `board` and
/// `id` are always written, empty when the kind has none, so an older binary still parses a file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartRef {
    /// The slug of a board.
    #[serde(default)]
    pub board: String,
    /// A gate's, a task's, a request's or a session's id, unique only within the board.
    #[serde(default)]
    pub id: String,
    /// A parent issue, by `repos[].parents[].key` of `adjutant_work`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// A pull request, `owner/repo#N`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// A point of a read-through, counted from 1 in the order it lists them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<u32>,
    /// The level of the points of a read-through to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<u32>,
    /// `right` (the new file) or `left` (the old one), for a range of a diff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl PartRef {
    /// A reference by a board and an id: a gate, a task, a session, a request.
    pub fn of(board: &str, id: &str) -> Self {
        PartRef {
            board: board.to_string(),
            id: id.to_string(),
            ..Default::default()
        }
    }
}

pub(super) fn display_path(root: &Path) -> PathBuf {
    root.join("aide-display.json")
}

pub(super) fn lock_path(root: &Path) -> PathBuf {
    root.join("aide-display.lock")
}

/// The display area as it is on disk. Nothing shown yet is an empty one; a file that is there
/// and cannot be read is an error, so the page says so rather than showing an area the aide has
/// filled as empty.
pub fn display(root: &Path) -> Result<AideDisplay, String> {
    read(&display_path(root))
}

pub(super) fn read(path: &Path) -> Result<AideDisplay, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AideDisplay::default()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}
