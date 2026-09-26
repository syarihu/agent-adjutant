//! A task the dashboard was handed, as a record that outlives the message announcing it.
//!
//! The message in the inbox is the *notification* — it is read once and acked, and after
//! that the hub has no way to say what became of the thing. The record is what the board
//! reads: one file per task, rewritten in place as the work moves. Both are derived from
//! this module so that only one of them is authored: `render_request` builds the message
//! body out of the same struct the file holds, rather than a second description of a task
//! kept in step by hand.
//!
//! A leaf: it is handed the directory to work in rather than deriving it, so it never has
//! to know where this machine keeps its state.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the human is asking for. The four shapes the hub's own procedure already sorts a
/// request into ("人間に話しかけられたら" 2/3/4/5) — named here so the form can ask once
/// instead of the hub inferring it from prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// An issue that already exists. Start on it.
    Start,
    /// No issue yet. File one, then start.
    FileAndStart,
    /// Report back; file nothing, move no board.
    Investigate,
    /// A postscript for a worker that is already running.
    TellWorker,
}

/// Where the worker stops. The vocabulary the brief's 完了条件 line already uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoneWhen {
    ReportOnly,
    Verify,
    Pr,
    Review,
}

/// Which gates wait on a person. The rest are kept as records the worker leaves and carries
/// on past. Chosen by whoever hands the task over, because whether anybody wants to look at
/// the diff or the check depends on the task, and the worker has no way to tell.
///
/// Each step includes the one before it: nobody asks to look at the check but not the diff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopAt {
    /// The plan only. The default, and what every task did before there was a choice.
    #[default]
    Plan,
    /// The plan and the diff.
    Diff,
    /// The plan, the diff and the check.
    All,
}

impl StopAt {
    pub fn as_str(self) -> &'static str {
        match self {
            StopAt::Plan => "plan",
            StopAt::Diff => "diff",
            StopAt::All => "all",
        }
    }
}

/// Who writes the code once the plan is approved.
///
/// A worker plans every task either way: the plan is where a strong model earns its keep,
/// and an agent that implements well from a detailed design does not need to write one.
/// What changes is what the worker does after the plan gate — implement it, or hand the
/// approved plan to Jules and stop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Executor {
    /// The worker implements, reviews and opens the pull request itself.
    #[default]
    Worker,
    /// The worker hands the approved plan to a Jules session, which implements it and opens
    /// the pull request.
    Jules,
}

impl Executor {
    pub fn parse(text: &str) -> Option<Executor> {
        Some(match text {
            "worker" => Executor::Worker,
            "jules" => Executor::Jules,
            _ => return None,
        })
    }

    fn is_worker(&self) -> bool {
        *self == Executor::Worker
    }
}

/// Six states, and no more.
///
/// There is deliberately no `gate` here. Whether a task is waiting on a human is answered
/// by whether an open gate exists for it, and adding a seventh state would mean the worker
/// has to remember to write it on the way in *and* on the way out — two writes that can
/// disagree with the gate directory, which is the thing actually being described.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Written down, not handed over. Nothing is in the inbox yet.
    Backlog,
    /// Handed to the hub. Waiting for it to pick the task up.
    Queued,
    /// A worker is on it.
    Dispatched,
    /// A pull request is open and the work is out of the worker's hands.
    Pr,
    Done,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Backlog => "backlog",
            Status::Queued => "queued",
            Status::Dispatched => "dispatched",
            Status::Pr => "pr",
            Status::Done => "done",
            Status::Cancelled => "cancelled",
        }
    }

    pub fn parse(text: &str) -> Option<Status> {
        Some(match text {
            "backlog" => Status::Backlog,
            "queued" => Status::Queued,
            "dispatched" => Status::Dispatched,
            "pr" => Status::Pr,
            "done" => Status::Done,
            "cancelled" => Status::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_url: Option<String>,
    pub done_when: DoneWhen,
    /// Absent in a record written before there was a choice, which is what `Plan` means.
    #[serde(default)]
    pub stop_at: StopAt,
    /// Absent means the worker, which is what every record written before there was a
    /// choice did. Left out when it is the worker, so those records read back unchanged.
    #[serde(default, skip_serializing_if = "Executor::is_worker")]
    pub executor: Executor,
    /// What this one dispatch should branch from. `None` = the repository's `baseBranch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The parent task's URL. A key would make the worker look the tracker up; a URL says
    /// which tracker it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Needed only when there is no issue to take a name from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    /// Whether the hub may dispatch this without asking. `false` becomes a question for the
    /// human rather than a halt: the hub is not allowed to block on one.
    pub auto_start: bool,
    /// Position in the queue. The human owns this; the hub reads it to pick what is next.
    #[serde(default)]
    pub order: u32,
    pub status: Status,
    // ── filled in as the work moves ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// The Jules session implementing this task, once the worker has handed it over. The id
    /// alone: the board asks the API for the state, which changes long after the worker has
    /// gone, rather than keeping a copy here that would go stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jules_session: Option<String>,
    /// The GitHub account that started the session, as `gh` named it then. Jules answers that
    /// account's comments and nobody else's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jules_by: Option<String>,
    /// Review comments already passed on to Jules, by their GitHub id. Kept so the board can
    /// say which ones went, and so one comment is not handed over twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relayed: Vec<String>,
    /// Review comments the board has told the hub about, so each one is brought up once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub announced: Vec<String>,
    /// How many times the board has told the hub about new review comments on this task's PR.
    /// Past a limit it stops, and passing comments on is left to a person: a reviewer and Jules
    /// answering each other's pushes can otherwise go round without end.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub relay_rounds: u32,
    /// Why the hub could not take it, when that is the answer. Written where the reply to
    /// the requester would have gone, because for a dashboard request there is no session
    /// to reply to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Handover instruction for the agent when queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    /// When a gate for this task was last answered or closed. The board counts the worker's
    /// time in a phase from here when it is later than the phase's start: waiting on a person
    /// is not the worker being stuck. Kept on the record, written as the gate is answered, so
    /// the board does not have to read the whole archive of answered gates on every poll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_answered_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Where this hub's tasks live. Beside the inbox rather than inside it: the inbox is a
/// queue that drains, and a task record has to still be there after its message is acked.
pub fn dir(state_dir: &Path, slug: &str) -> PathBuf {
    state_dir.join("tasks").join(slug)
}

/// `20260922T041233Z-login-retry`. The stamp comes from the caller so this stays a leaf —
/// and so a test can pin it.
pub fn new_id(stamp: &str, title: &str) -> String {
    let slug = slugify(title);
    if slug.is_empty() {
        stamp.to_string()
    } else {
        format!("{stamp}-{slug}")
    }
}

/// Lower-cased ASCII words joined by hyphens, cut short. Anything else — Japanese, most
/// punctuation — is a separator rather than transliterated: a filename is not where a title
/// is preserved, and the title itself is right there in the record.
fn slugify(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 32 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn path_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Take an id nobody else holds, and hold it.
///
/// The stamp has one-second resolution and a Japanese title slugs to nothing, so two tasks
/// written in the same second are not a rare case — it is what filling the form twice looks
/// like, and the second one would land on the first one's file and erase it.
///
/// Claimed with `create_new` rather than checked with `exists` first: the dashboard and the
/// command line can both be creating one, and check-then-write leaves a window where both
/// see the name free. `messaging::send` names inbox files the same way, for the same reason.
pub fn claim_id(dir: &Path, stamp: &str, title: &str) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let base = new_id(stamp, title);
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
                    "cannot create a task file in {}: {e}",
                    dir.display()
                ));
            }
        }
    }
    Err(format!("no free task id for {base}"))
}

/// Write the record, creating the directory if this is the first one.
///
/// Written whole each time rather than patched: every caller already holds the struct it
/// wants on disk, and a partial write is how two writers end up with a record neither of
/// them would recognise.
pub fn save(dir: &Path, task: &Task) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = path_of(dir, &task.id);
    let json = serde_json::to_string_pretty(task).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

pub fn load(dir: &Path, id: &str) -> Result<Task, String> {
    let path = path_of(dir, id);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no such task: {id}"))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Every task this hub knows about, queue order first.
///
/// A file that will not parse is skipped rather than fatal. The board is a view of a
/// directory somebody may have hand-edited, and one bad file must not blank the page.
pub fn list(dir: &Path) -> Vec<Task> {
    let mut tasks: Vec<Task> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Task>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    tasks.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    tasks
}

/// The body of the `request` message that tells the hub this task exists.
///
/// Derived from the record rather than typed beside it: the server writes the file and then
/// renders this, so the hub can work from `adj pending --read` alone without opening the
/// record, and the two can never drift.
pub fn render_request(task: &Task) -> String {
    let mut out = String::new();
    out.push_str(&format!("## task        {}\n", task.id));
    out.push_str(&format!(
        "## 種類        {}\n",
        match task.kind {
            Kind::Start => "Issue に着手",
            Kind::FileAndStart => "起票して着手",
            Kind::Investigate => "調査だけ(報告して終わり)",
            Kind::TellWorker => "既存 worktree へ追加指示",
        }
    ));
    out.push_str(&format!(
        "## 完了条件     {}\n",
        match task.done_when {
            DoneWhen::ReportOnly => "調査のみ(報告して終わり)",
            DoneWhen::Verify => "動作確認まで",
            DoneWhen::Pr => "PR 作成まで",
            DoneWhen::Review => "レビュー対応まで",
        }
    ));
    // The value itself goes first: the hub passes it on as `--stop-at` and into the brief,
    // and the gloss is for whoever reads the message.
    out.push_str(&format!(
        "## 止める所     {}（{}）\n",
        task.stop_at.as_str(),
        match task.stop_at {
            StopAt::Plan => "計画の承認だけ待つ",
            StopAt::Diff => "計画の承認と差分レビューを待つ",
            StopAt::All => "計画の承認・差分レビュー・動作確認を待つ",
        }
    ));
    // Only when it is not the worker, so a request reads the way it always has for the tasks
    // that did not choose.
    if task.executor == Executor::Jules {
        out.push_str("## 実装        jules（計画の承認後に Jules へ渡す）\n");
    }
    let line = |label: &str, value: Option<&str>| format!("## {label}{}\n", value.unwrap_or("-"));
    out.push_str(&line("Issue      ", task.issue_url.as_deref()));
    out.push_str(&line("分岐元      ", task.base.as_deref()));
    out.push_str(&line("親タスク    ", task.parent.as_deref()));
    out.push_str(&line("worktree名 ", task.worktree_name.as_deref()));
    out.push_str(&format!(
        "## 着手        {}\n",
        if task.auto_start {
            "確認なしで着手してよい"
        } else {
            "着手前に確認がほしい"
        }
    ));
    out.push_str("\n## 内容\n\n");
    out.push_str(task.body.trim_end());
    out.push('\n');
    if let Some(instruction) = task.instruction.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push_str("\n## 申し送り\n\n");
        out.push_str(instruction.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Production builds a task out of the form's JSON; this is the shorthand the tests
    /// need and nothing else does.
    impl Task {
        /// A task as the form hands it over. Everything the hub fills in later is absent.
        pub fn new(
            id: String,
            kind: Kind,
            title: String,
            done_when: DoneWhen,
            stamp: &str,
        ) -> Task {
            Task {
                id,
                kind,
                title,
                body: String::new(),
                issue_url: None,
                done_when,
                stop_at: StopAt::default(),
                executor: Executor::default(),
                base: None,
                parent: None,
                worktree_name: None,
                auto_start: true,
                order: 0,
                status: Status::Backlog,
                worktree: None,
                issue: None,
                pr: None,
                jules_session: None,
                jules_by: None,
                relayed: Vec::new(),
                announced: Vec::new(),
                relay_rounds: 0,
                note: None,
                instruction: None,
                gate_answered_at: None,
                created_at: stamp.to_string(),
                updated_at: stamp.to_string(),
            }
        }
    }

    fn sample() -> Task {
        let mut task = Task::new(
            new_id("20260922T041233Z", "Fix the login retry"),
            Kind::Investigate,
            "ログインのリトライを調べる".to_string(),
            DoneWhen::ReportOnly,
            "20260922T041233Z",
        );
        task.body = "リトライが効いていない気がするのだ".to_string();
        task
    }

    #[test]
    fn id_carries_a_readable_tail_when_the_title_has_one() {
        assert_eq!(
            new_id("20260922T041233Z", "Fix the login retry"),
            "20260922T041233Z-fix-the-login-retry"
        );
    }

    /// A Japanese title leaves nothing to slugify, and the stamp alone is still a usable
    /// id. The alternative — refusing the task — would reject the common case here.
    #[test]
    fn id_is_the_stamp_alone_when_nothing_survives_slugging() {
        assert_eq!(
            new_id("20260922T041233Z", "ログインのリトライ"),
            "20260922T041233Z"
        );
    }

    #[test]
    fn slug_does_not_run_past_its_cap_or_end_on_a_separator() {
        let slug = slugify("a very long english title that keeps going and going and going");
        assert!(slug.len() <= 32, "{slug}");
        assert!(!slug.ends_with('-'), "{slug}");
    }

    /// Two tasks written in the same second, with titles that slug to nothing, must not
    /// land on the same file — which is what filling the form twice in a row looks like.
    #[test]
    fn ids_claimed_in_the_same_second_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3)
            .map(|_| claim_id(dir.path(), "20260922T041233Z", "ログインのリトライ").unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "20260922T041233Z",
                "20260922T041233Z-2",
                "20260922T041233Z-3"
            ]
        );
    }

    /// The claim leaves the file behind, so `save` has somewhere to land and no second
    /// caller can take the name in between.
    #[test]
    fn a_claimed_id_is_held_before_anything_is_written_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let id = claim_id(dir.path(), "20260922T041233Z", "x").unwrap();
        assert!(path_of(dir.path(), &id).exists());
    }

    #[test]
    fn a_saved_task_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let task = sample();
        save(dir.path(), &task).unwrap();
        assert_eq!(load(dir.path(), &task.id).unwrap(), task);
    }

    #[test]
    fn listing_is_in_queue_order() {
        let dir = tempfile::tempdir().unwrap();
        for (id, order) in [("a", 3u32), ("b", 1), ("c", 2)] {
            let mut task = sample();
            task.id = id.to_string();
            task.order = order;
            save(dir.path(), &task).unwrap();
        }
        let ids: Vec<String> = list(dir.path()).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }

    /// One unreadable file must not blank the board.
    #[test]
    fn a_file_that_will_not_parse_is_skipped_rather_than_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let task = sample();
        save(dir.path(), &task).unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ not json").unwrap();
        assert_eq!(list(dir.path()).len(), 1);
    }

    #[test]
    fn listing_a_directory_that_is_not_there_is_empty_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("nope")).is_empty());
    }

    /// The message the hub reads has to carry everything the form asked for; otherwise the
    /// hub goes back to asking the questions the form existed to answer.
    #[test]
    fn the_request_body_carries_what_the_form_collected() {
        let mut task = sample();
        task.base = Some("origin/release/1.2".to_string());
        task.worktree_name = Some("login-retry".to_string());
        task.auto_start = false;
        let body = render_request(&task);
        assert!(body.contains(&task.id), "{body}");
        assert!(body.contains("origin/release/1.2"), "{body}");
        assert!(body.contains("login-retry"), "{body}");
        assert!(body.contains("着手前に確認がほしい"), "{body}");
        assert!(body.contains("調査だけ(報告して終わり)"), "{body}");
        assert!(
            body.contains("リトライが効いていない気がするのだ"),
            "{body}"
        );
    }

    /// Absent fields are written as `-` rather than left out: the hub reads this as prose,
    /// and a missing line reads as "nobody said" while an empty one reads as "said nothing".
    #[test]
    fn unset_fields_say_so_rather_than_vanishing() {
        let body = render_request(&sample());
        assert!(body.contains("## 分岐元      -"), "{body}");
        assert!(body.contains("## 親タスク    -"), "{body}");
    }

    /// The hub copies the stop point into the brief, so the message has to say it — and say
    /// the default out loud rather than leave the line off.
    #[test]
    fn the_request_body_says_where_the_task_stops() {
        let mut task = sample();
        assert!(
            render_request(&task).contains("## 止める所     plan（"),
            "{}",
            render_request(&task)
        );
        task.stop_at = StopAt::All;
        assert!(
            render_request(&task).contains("## 止める所     all（"),
            "{}",
            render_request(&task)
        );
    }

    /// A record written before the field existed stopped at the plan, and still does.
    #[test]
    fn a_record_without_a_stop_point_stops_at_the_plan() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value.as_object_mut().unwrap().remove("stopAt");
        let task: Task = serde_json::from_value(value).unwrap();
        assert_eq!(task.stop_at, StopAt::Plan);
    }

    #[test]
    fn the_request_body_includes_instruction_when_present() {
        let mut task = sample();
        assert!(!render_request(&task).contains("## 申し送り"));

        task.instruction = Some("まずは既存コードの挙動を調査してほしいのだ".to_string());
        let body = render_request(&task);
        assert!(body.contains("## 申し送り\n\nまずは既存コードの挙動を調査してほしいのだ\n"));
    }

    #[test]
    fn a_record_without_an_instruction_deserializes_with_none() {
        let value = serde_json::to_value(sample()).unwrap();
        let task: Task = serde_json::from_value(value).unwrap();
        assert_eq!(task.instruction, None);
    }
}
