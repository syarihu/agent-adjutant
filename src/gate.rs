//! A gate: something an agent has prepared for a person to look at, and the ball handed
//! over with it.
//!
//! Not a question with buttons. The agent stops, puts the artefact where a person can see
//! it, and waits — so the shape of the payload is the point, and it is three frames rather
//! than one wall of prose: what to look at, what was already decided, and what the agent
//! was unsure of. A reviewer who has to read four hundred lines to find the two decisions
//! that matter is a reviewer who approves without reading.
//!
//! The answer travels back out through the outbox the hub already uses to reach a worker,
//! so a gate needs no channel of its own — and an answer to a worktree whose worker has
//! died simply waits there, which is the same promise `adjutant tell` already makes.
//!
//! A leaf: handed the directory to work in, and told the time rather than asking.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What is being shown. Each one is a moment that used to be an `AskUserQuestion` in a tab
/// nobody was watching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A worker's plan, before it writes anything.
    Plan,
    /// A diff, once the worker's own review rounds have converged.
    Diff,
    /// Built and ready; somebody has to run it. The one kind whose answer comes after work
    /// that happens outside the browser.
    Verify,
    /// The hub asking whether to start on something.
    Dispatch,
    /// The hub's draft of an issue it is about to file.
    Issue,
    /// Neither of the above: something an agent cannot decide by itself.
    Question,
    /// A finished piece of investigation. There is nothing to approve — it is read.
    Result,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Plan => "plan",
            Kind::Diff => "diff",
            Kind::Verify => "verify",
            Kind::Dispatch => "dispatch",
            Kind::Issue => "issue",
            Kind::Question => "question",
            Kind::Result => "result",
        }
    }

    /// Whether the hub opened this rather than a worker. The hub reads its inbox and never
    /// an outbox, so the answer has to be delivered there instead — to the outbox of the
    /// main checkout it would sit unread.
    ///
    /// Decided by kind because the kind already says who is asking: whether to start a task
    /// and whether to file an issue are the hub's questions, and nothing a worker asks about.
    /// A kind that either side could open would need the opener written into the gate.
    pub fn answered_by_hub(self) -> bool {
        matches!(self, Kind::Dispatch | Kind::Issue)
    }

    /// Whether a gate of this kind may be kept as a record instead of waited on.
    ///
    /// Only the two a worker opens after its own checks have run: the review and the check
    /// can have nothing in them for a person. A plan always waits, and the hub's and the
    /// question kinds exist to be answered.
    pub fn can_be_recorded(self) -> bool {
        matches!(self, Kind::Diff | Kind::Verify)
    }

    /// What the buttons are, when the payload does not say.
    pub fn default_options(self) -> Vec<String> {
        let options: &[&str] = match self {
            Kind::Result => &["ack", "ask"],
            Kind::Verify | Kind::Diff => &["approve", "changes"],
            Kind::Question => &["answer"],
            _ => &["approve", "changes", "reject"],
        };
        options.iter().map(|o| o.to_string()).collect()
    }
}

/// How bad a review finding is. The same three words the worker's review loop reports in, so
/// a record reads the way the rounds were run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// A defect if merged.
    Must,
    /// Correct, and could be better.
    Want,
    /// Not what the task asked for.
    Scope,
}

/// What became of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Still there: not fixed, and not ruled out.
    Open,
    Fixed,
    /// Checked against the code and ruled a false positive. The reason goes with it.
    Declined,
}

/// Why a `diff` or `verify` gate waits on a person rather than being kept as a record. The
/// worker's procedure names the same five rules, and a gate that stops it says which fired,
/// so the board can tell a stop that needs a person from one the task was handed over with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopRule {
    /// The review rounds hit `selfReviewRounds` with a must still open.
    RoundLimit,
    /// `verify` failed and the worker could not fix it.
    VerifyFailed,
    /// Something only a person can check, such as a screen change.
    ManualCheck,
    /// The worker wrote down where its confidence ran out.
    Unsure,
    /// The task's stop point covers this gate.
    StopAt,
}

impl StopRule {
    pub fn as_str(self) -> &'static str {
        match self {
            StopRule::RoundLimit => "round-limit",
            StopRule::VerifyFailed => "verify-failed",
            StopRule::ManualCheck => "manual-check",
            StopRule::Unsure => "unsure",
            StopRule::StopAt => "stop-at",
        }
    }

    /// Whether this rule can be why a gate of `kind` stopped. The same table the worker's
    /// procedure decides by: the review's round limit is the diff's, a check left for a
    /// person is the verify's, and the rest can stop either.
    pub fn applies_to(self, kind: Kind) -> bool {
        match self {
            StopRule::RoundLimit => kind == Kind::Diff,
            StopRule::ManualCheck => kind == Kind::Verify,
            StopRule::VerifyFailed | StopRule::Unsure | StopRule::StopAt => kind.can_be_recorded(),
        }
    }
}

/// One round of the worker's own review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRound {
    /// Who read the diff: `claude`, `codex`, ...
    pub engine: String,
    #[serde(default)]
    pub must: u32,
    #[serde(default)]
    pub want: u32,
    #[serde(default)]
    pub scope: u32,
    #[serde(default)]
    pub false_positives: u32,
}

/// One thing a review round raised, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    /// `file:line`, or whatever the reviewer pointed at.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,
    pub text: String,
    pub outcome: Outcome,
    /// Why it was declined. A false positive with no reason is one the next reader has to
    /// re-check from scratch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunResult {
    Pass,
    Fail,
}

/// One `verify` command as it was run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
    pub command: String,
    pub result: RunResult,
    /// How long it took, as the worker measured it (`42s`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// What it printed, or the tail of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// How many runs it took to reach `result`. Above one, it failed first and was fixed: a
    /// pass the board marks, since the first run is the one that says something was wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
}

/// One answer to a record. A record stays where it is when it is answered, so the answers
/// are appended rather than written over the gate's own `decision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub answered_at: String,
}

/// One of the designs an agent is asking a person to choose between.
///
/// Two of these side by side is the thing a terminal cannot do: in a tab the second option
/// has already scrolled past the first by the time you have read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<String>,
    /// Which one the agent would pick. Said out loud rather than implied by ordering.
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gate {
    pub id: String,
    pub kind: Kind,
    /// Where the answer goes. A worktree rather than a session, so an answer outlives the
    /// agent that asked for it.
    pub worktree: String,
    /// The task this belongs to, when there is a record for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub title: String,
    /// What is true regardless of the decision: rounds run, tests passed, files touched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<String>,
    // ── the three frames ──
    /// What the person has to decide. Read this and you can answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// Settled. Shown folded away, because it is here to be available rather than read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided: Option<String>,
    /// Where the agent's own confidence ran out. The half of a review people never get.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsure: Option<String>,
    // ── the attachments, by kind ──
    /// A report, for a gate that is read rather than decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// How to run it, for `verify`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// How many times this same point has been round-tripped. The agent knows; the board
    /// uses it to suggest going to the tab instead, because past two rounds a gate has
    /// become a conversation and a conversation is faster where it is not posted.
    #[serde(default)]
    pub rounds: u32,
    // ── the structured attachments, by kind ──
    /// `plan`: what is wrong today, from the request and the issue the worker read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// `plan`: what done looks like.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// `diff`: the worker's own review, one entry per round. Not `rounds`, which already
    /// counts how often this gate has been round-tripped with a person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_rounds: Vec<ReviewRound>,
    /// `diff`: what those rounds raised.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// `verify`: the commands that were run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandRun>,
    /// `verify`: the checks left for a person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manual: Vec<String>,
    /// `diff` / `verify`: the rules that made this gate wait rather than be kept as a record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stopped_by: Vec<StopRule>,
    /// `false` for a record: written down for the board, while the worker carries on.
    #[serde(default = "waits", skip_serializing_if = "is_waiting")]
    pub wait: bool,
    pub opened_at: String,
    // ── written when it is answered ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<String>,
    /// A record's answers, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<Answer>,
}

fn waits() -> bool {
    true
}

fn is_waiting(wait: &bool) -> bool {
    *wait
}

/// Where this hub's open gates wait.
pub fn dir(state_dir: &Path, slug: &str) -> PathBuf {
    state_dir.join("gates").join(slug)
}

/// Where a gate goes once it has been answered. Kept rather than deleted, for the same
/// reason the inbox keeps what it has read: a decision that turned out wrong has to be
/// findable afterwards.
pub fn answered_dir(state_dir: &Path, slug: &str) -> PathBuf {
    dir(state_dir, slug).join("answered")
}

/// Where a gate kept as a record lives. Beside `answered/` rather than in the open queue, so
/// nothing that counts what is waiting for a person counts it.
pub fn records_dir(state_dir: &Path, slug: &str) -> PathBuf {
    dir(state_dir, slug).join("records")
}

pub fn path_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// `20260922T041233Z-diff`, and a suffix if that name is taken.
///
/// Claimed rather than checked, like a task id and an inbox filename: a worker finishing two
/// pieces of work in the same second is ordinary, and the loser of a check-then-write would
/// overwrite a gate somebody is in the middle of reading.
pub fn claim_id(dir: &Path, stamp: &str, kind: Kind) -> Result<String, String> {
    claim(dir, format!("{stamp}-{}", kind.as_str()))
}

/// `20260922T041233Z-diff-record`. Named apart from an open gate's id rather than claimed in
/// both directories: an answer finds its gate by id alone, and a record and a gate opened in
/// the same second must not be mistaken for each other.
pub fn claim_record_id(dir: &Path, stamp: &str, kind: Kind) -> Result<String, String> {
    claim(dir, format!("{stamp}-{}-record", kind.as_str()))
}

fn claim(dir: &Path, base: String) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for seq in 1..1000 {
        let id = if seq == 1 {
            base.clone()
        } else {
            format!("{base}-{seq}")
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path_of(dir, &id))
        {
            Ok(_) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "cannot create a gate file in {}: {e}",
                    dir.display()
                ));
            }
        }
    }
    Err(format!("no free gate id for {base}"))
}

/// Write a gate, replacing whatever is at its name in one step.
///
/// A record is written again each time it is answered, while the board reads it every few
/// seconds and nothing it reads through takes the writer's lock. Written in place, a reader
/// could catch it half-written and drop it from the listing, and a write cut short would
/// leave it unreadable for good. Staged beside it and renamed over it, the name never points
/// at a partial file.
pub fn save(dir: &Path, gate: &Gate) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = path_of(dir, &gate.id);
    let json = serde_json::to_string_pretty(gate).map_err(|e| e.to_string())?;
    // Not named `.json`, so a listing never reads a staged file as a gate. The pid keeps two
    // processes writing the same gate from staging into each other's file.
    let staged = dir.join(format!(".{}.{}.tmp", gate.id, std::process::id()));
    std::fs::write(&staged, format!("{json}\n"))
        .map_err(|e| format!("cannot write {}: {e}", staged.display()))?;
    std::fs::rename(&staged, &path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("cannot write {}: {e}", path.display())
    })?;
    Ok(path)
}

pub fn load(dir: &Path, id: &str) -> Result<Gate, String> {
    let path = path_of(dir, id);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no open gate: {id}"))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Every gate still waiting for a person, oldest first — which is the order they should be
/// worked through. Given the records directory instead, every record, in the same order.
pub fn list(dir: &Path) -> Vec<Gate> {
    list_where(dir, |_| true)
}

/// The gates of one kind, told apart by their file names before any is read. For the
/// archive, which only grows: the board asks it for plans on every poll, and parsing every
/// diff ever answered to find them would cost more each day.
pub fn list_of_kind(dir: &Path, kind: Kind) -> Vec<Gate> {
    // `{stamp}-{kind}`, `{stamp}-{kind}-{seq}` or `{stamp}-{kind}-record`. No kind's name
    // begins another's, so the prefix is enough; the kind is checked again once parsed.
    list_where(dir, |id| {
        id.split_once('-')
            .is_some_and(|(_, rest)| rest.starts_with(kind.as_str()))
    })
    .into_iter()
    .filter(|g| g.kind == kind)
    .collect()
}

fn list_where(dir: &Path, wanted: impl Fn(&str) -> bool) -> Vec<Gate> {
    let mut gates: Vec<Gate> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter(|e| {
                e.path()
                    .file_stem()
                    .is_some_and(|stem| wanted(&stem.to_string_lossy()))
            })
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Gate>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    gates.sort_by(|a, b| a.opened_at.cmp(&b.opened_at));
    gates
}

/// Move an answered gate out of the way, so "open" means what it says.
pub fn archive(dir: &Path, answered: &Path, gate: &Gate) -> Result<PathBuf, String> {
    std::fs::create_dir_all(answered)
        .map_err(|e| format!("cannot create {}: {e}", answered.display()))?;
    let path = path_of(answered, &gate.id);
    let json = serde_json::to_string_pretty(gate).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let _ = std::fs::remove_file(path_of(dir, &gate.id));
    Ok(path)
}

/// The subject the answer is delivered under.
///
/// The identifier goes first, exactly as the hub's own `[質問 {stamp}]` does: it is the
/// only thing tying an answer to what was asked, and an agent reading its outbox has
/// nothing else to match on.
pub fn answer_subject(gate: &Gate, decision: &str) -> String {
    format!("[gate {}] {decision}", gate.id)
}

/// What the worker reads in its outbox.
///
/// Written as prose rather than JSON because the reader is an agent mid-task: it has to be
/// obvious what was decided from the first line, and the comment is the part that changes
/// what happens next.
pub fn answer_body(
    gate: &Gate,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
) -> String {
    let mut out = format!("## 判定        {decision}\n");
    if let Some(choice) = choice {
        let label = gate
            .choices
            .iter()
            .find(|c| c.id == choice)
            .map(|c| c.label.as_str())
            .unwrap_or(choice);
        out.push_str(&format!("## 選ばれた案   {label}({choice})\n"));
    }
    // A record said as one, so the worker knows the person went back to something it had
    // already moved past rather than something it is waiting on.
    let record = if gate.wait { "" } else { ", 記録" };
    out.push_str(&format!(
        "## gate       {} ({}{record})\n",
        gate.id,
        gate.kind.as_str()
    ));
    // By the time this is read the gate has been archived, so `adj gate show` no longer
    // finds it. The hub has to know which task it just decided on from the message alone.
    if let Some(task) = &gate.task {
        out.push_str(&format!("## task       {task}\n"));
    }
    match comment.map(str::trim).filter(|c| !c.is_empty()) {
        Some(comment) => out.push_str(&format!("\n## コメント\n\n{comment}\n")),
        // Said rather than left out: an agent that sees no comment section has to work out
        // whether there was none or whether it lost one.
        None => out.push_str("\n## コメント\n\n(なし)\n"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(kind: Kind) -> Gate {
        Gate {
            id: "20260922T041233Z-plan".to_string(),
            kind,
            worktree: "/tmp/wt".to_string(),
            task: Some("20260922T041000Z-cache".to_string()),
            title: "設計レビュー: 検索結果のキャッシュ".to_string(),
            facts: vec!["触る予定のファイル 6 件".to_string()],
            focus: Some("TTL の持ち方を決めてほしいのだ".to_string()),
            decided: Some("LRU 64件にするのだ".to_string()),
            unsure: None,
            body: None,
            run: None,
            diff: None,
            choices: vec![Choice {
                id: "const".to_string(),
                label: "案A — 定数で持つ".to_string(),
                why: "remote config が無いのだ".to_string(),
                points: vec!["差分 小".to_string()],
                recommended: true,
            }],
            options: vec!["approve".to_string(), "changes".to_string()],
            rounds: 0,
            problem: None,
            goal: None,
            review_rounds: Vec::new(),
            findings: Vec::new(),
            commands: Vec::new(),
            manual: Vec::new(),
            stopped_by: Vec::new(),
            wait: true,
            opened_at: "20260922T041233Z".to_string(),
            decision: None,
            choice: None,
            comment: None,
            answered_at: None,
            answers: Vec::new(),
        }
    }

    #[test]
    fn a_saved_gate_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let gate = gate(Kind::Plan);
        save(dir.path(), &gate).unwrap();
        assert_eq!(load(dir.path(), &gate.id).unwrap(), gate);
    }

    /// Written again over itself, a gate leaves nothing staged behind and still reads back.
    #[test]
    fn saving_over_a_gate_replaces_it_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let mut gate = gate(Kind::Diff);
        save(dir.path(), &gate).unwrap();
        gate.title = "書き直した".to_string();
        save(dir.path(), &gate).unwrap();
        assert_eq!(load(dir.path(), &gate.id).unwrap(), gate);
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, [format!("{}.json", gate.id)]);
    }

    /// Two pieces of work finishing in the same second is ordinary, and the loser of a
    /// check-then-write would overwrite a gate somebody is reading.
    #[test]
    fn ids_claimed_in_the_same_second_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3)
            .map(|_| claim_id(dir.path(), "20260922T041233Z", Kind::Diff).unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "20260922T041233Z-diff",
                "20260922T041233Z-diff-2",
                "20260922T041233Z-diff-3"
            ]
        );
    }

    #[test]
    fn listing_is_oldest_first_so_the_queue_is_worked_in_order() {
        let dir = tempfile::tempdir().unwrap();
        for (id, at) in [
            ("c", "20260922T03"),
            ("a", "20260922T01"),
            ("b", "20260922T02"),
        ] {
            let mut gate = gate(Kind::Plan);
            gate.id = id.to_string();
            gate.opened_at = at.to_string();
            save(dir.path(), &gate).unwrap();
        }
        let ids: Vec<String> = list(dir.path()).into_iter().map(|g| g.id).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn listing_by_kind_reads_only_that_kind() {
        let dir = tempfile::tempdir().unwrap();
        for (id, kind) in [
            ("20260922T01Z-plan", Kind::Plan),
            ("20260922T02Z-plan-2", Kind::Plan),
            ("20260922T03Z-diff", Kind::Diff),
        ] {
            let mut gate = gate(kind);
            gate.id = id.to_string();
            save(dir.path(), &gate).unwrap();
        }
        let ids: Vec<String> = list_of_kind(dir.path(), Kind::Plan)
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(ids.iter().all(|id| id.contains("-plan")), "{ids:?}");
    }

    /// The `answered` subdirectory lives inside the gate directory, so it must not read as
    /// a gate itself.
    #[test]
    fn the_archive_is_not_listed_as_an_open_gate() {
        let dir = tempfile::tempdir().unwrap();
        let gate = gate(Kind::Plan);
        save(dir.path(), &gate).unwrap();
        let answered = dir.path().join("answered");
        archive(dir.path(), &answered, &gate).unwrap();
        assert!(list(dir.path()).is_empty());
        assert!(path_of(&answered, &gate.id).exists());
        assert!(!path_of(dir.path(), &gate.id).exists());
    }

    #[test]
    fn a_broken_file_is_skipped_rather_than_blanking_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &gate(Kind::Plan)).unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ not json").unwrap();
        assert_eq!(list(dir.path()).len(), 1);
    }

    /// The identifier leads, because it is the only thing an agent reading its outbox can
    /// match an answer against.
    #[test]
    fn the_subject_leads_with_the_gate_id() {
        assert_eq!(
            answer_subject(&gate(Kind::Plan), "approve"),
            "[gate 20260922T041233Z-plan] approve"
        );
    }

    #[test]
    fn the_answer_body_names_the_decision_and_the_chosen_option() {
        let body = answer_body(
            &gate(Kind::Plan),
            "choice",
            Some("const"),
            Some("これで進めてほしいのだ"),
        );
        assert!(body.contains("## 判定        choice"), "{body}");
        assert!(body.contains("案A — 定数で持つ(const)"), "{body}");
        assert!(body.contains("これで進めてほしいのだ"), "{body}");
    }

    /// An agent that sees no comment section cannot tell "there was none" from "one was
    /// lost on the way".
    #[test]
    fn a_missing_comment_is_said_rather_than_left_out() {
        let body = answer_body(&gate(Kind::Diff), "approve", None, None);
        assert!(body.contains("## コメント\n\n(なし)"), "{body}");
    }

    #[test]
    fn a_record_s_id_is_told_apart_from_a_gate_s() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            claim_record_id(dir.path(), "20260922T041233Z", Kind::Diff).unwrap(),
            "20260922T041233Z-diff-record"
        );
    }

    /// The structured fields are what the board draws its tables from, so they have to come
    /// back out exactly as the worker wrote them.
    #[test]
    fn a_record_with_its_structured_fields_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let mut gate = gate(Kind::Diff);
        gate.wait = false;
        gate.review_rounds = vec![ReviewRound {
            engine: "claude".to_string(),
            must: 2,
            want: 1,
            scope: 0,
            false_positives: 1,
        }];
        gate.findings = vec![Finding {
            severity: Severity::Must,
            location: "src/gate.rs:10".to_string(),
            text: "unwrap on a missing file".to_string(),
            outcome: Outcome::Declined,
            reason: Some("the file is created just above".to_string()),
        }];
        gate.answers = vec![Answer {
            decision: "changes".to_string(),
            comment: Some("もう一度見てほしいのだ".to_string()),
            answered_at: "20260922T050000Z".to_string(),
        }];
        save(dir.path(), &gate).unwrap();
        assert_eq!(load(dir.path(), &gate.id).unwrap(), gate);

        let json = serde_json::to_value(&gate).unwrap();
        assert_eq!(json["wait"], false);
        assert_eq!(json["reviewRounds"][0]["falsePositives"], 1);
        assert_eq!(json["findings"][0]["outcome"], "declined");
    }

    /// A gate written before records existed has no `wait`, and waits.
    #[test]
    fn a_gate_without_wait_is_one_that_waits() {
        let mut json = serde_json::to_value(gate(Kind::Plan)).unwrap();
        assert!(json.get("wait").is_none(), "{json}");
        json.as_object_mut().unwrap().remove("wait");
        assert!(serde_json::from_value::<Gate>(json).unwrap().wait);
    }

    #[test]
    fn an_answer_to_a_record_says_it_is_one() {
        let mut gate = gate(Kind::Verify);
        gate.wait = false;
        let body = answer_body(&gate, "changes", None, Some("直してほしいのだ"));
        assert!(body.contains("(verify, 記録)"), "{body}");
    }

    /// A report is read, not approved, so it must not come with an Approve button.
    #[test]
    fn a_result_gate_offers_reading_rather_than_approving() {
        assert_eq!(Kind::Result.default_options(), ["ack", "ask"]);
        assert_eq!(Kind::Diff.default_options(), ["approve", "changes"]);
    }
}
