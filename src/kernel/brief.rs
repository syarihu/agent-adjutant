//! The worker's brief: `.claude/task-brief.md`, written from the task record and the config.
//!
//! The worker starts clean, so this file is everything it knows about where it stands. It
//! used to be filled in by hand from a template in the hub's procedure; written here, the
//! lines cannot drift from the record, and the label of each one is named once.
//!
//! A leaf: the labels are pinned in this file and cross-checked against the procedures that
//! read them in the `transport::cli::task` tests, which may see both sides. A new line is a field
//! on `TaskBrief`, a label in `label`, and one call in `render_task`.

use serde_json::{Map, Value};

/// The label each line of the brief starts with after `- `. The worker finds a line by its
/// label, so these are what it reads by.
pub mod label {
    pub const TASK: &str = "Task";
    pub const WORKSPACE: &str = "Workspace";
    pub const BASE_BRANCH: &str = "Base branch";
    pub const PARENT_TASK: &str = "Parent task";
    pub const TASK_RECORD: &str = "Task record";
    pub const DONE_WHEN: &str = "Done when";
    pub const STOP_AT: &str = "Stop at";
    pub const HANDOVER_NOTE: &str = "Handover note";
    pub const COPILOT_REVIEW: &str = "Copilot review";
    pub const LANGUAGE: &str = "Language";
    pub const REPORT_TO: &str = "Report to";
    pub const VERIFY_COMMANDS: &str = "Verify commands";

    /// Every line of a task brief, in the order written. Held by the tests, which is where
    /// the writer and the procedures are compared.
    #[cfg(test)]
    pub const TASK_BRIEF: [&str; 12] = [
        TASK,
        WORKSPACE,
        BASE_BRANCH,
        PARENT_TASK,
        TASK_RECORD,
        DONE_WHEN,
        STOP_AT,
        HANDOVER_NOTE,
        COPILOT_REVIEW,
        LANGUAGE,
        REPORT_TO,
        VERIFY_COMMANDS,
    ];

    /// The ones `adj-worker` names when it says which line decides something.
    #[cfg(test)]
    pub const READ_BY_WORKER: [&str; 9] = [
        TASK,
        BASE_BRANCH,
        PARENT_TASK,
        TASK_RECORD,
        DONE_WHEN,
        STOP_AT,
        HANDOVER_NOTE,
        COPILOT_REVIEW,
        LANGUAGE,
    ];
}

/// What a session with no instruction yet is told to do.
pub const NO_INSTRUCTION: &str = "No instruction yet. Greet the person in this tab, say you are ready, and wait for what they want.";

/// Which tool the worker reads the ticket with. `None` is a task with no issue, written `-`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tracker {
    Github,
    GithubProject,
    Jira,
    Linear,
    None,
}

impl Tracker {
    pub fn as_str(self) -> &'static str {
        match self {
            Tracker::Github => "github",
            Tracker::GithubProject => "github-project",
            Tracker::Jira => "jira",
            Tracker::Linear => "linear",
            Tracker::None => "-",
        }
    }

    /// The tracker a person names. Matched exactly, as the list it replaces was.
    pub fn parse(text: &str) -> Result<Tracker, String> {
        match text {
            "github" => Ok(Tracker::Github),
            "github-project" => Ok(Tracker::GithubProject),
            "jira" => Ok(Tracker::Jira),
            "linear" => Ok(Tracker::Linear),
            "-" => Ok(Tracker::None),
            _ => Err(format!(
                "no such tracker: {text} (github, github-project, jira, linear or -)"
            )),
        }
    }
}

/// Everything a task's brief says. Text the person or the tracker wrote is held as it was
/// given; `render_task` is what keeps it from passing for a line of its own.
pub struct TaskBrief {
    /// The tracker's key (`WID-12`), or `-` when the task has no issue.
    pub key: String,
    pub title: String,
    /// The `type` of the task's source: the worker reads the ticket with the tool it names.
    pub tracker: Tracker,
    pub url: Option<String>,
    /// The request text, written in place of the URL when there is none.
    pub request: String,
    pub branch: String,
    /// What the branch was cut from, or `-` when that is not known.
    pub base: String,
    /// The parent task's URL, or `-`.
    pub parent: String,
    /// The id of the board's card.
    pub record: String,
    pub done_when: String,
    pub stop_at: String,
    /// The handover note, or `-`.
    pub handover: String,
    pub copilot_review: String,
    /// The language the person reads, or `-` when that is not known.
    pub language: String,
    pub verify: Vec<String>,
}

/// The brief of a session that was started with no task.
pub struct SessionBrief {
    pub branch: String,
    pub base: String,
    /// The language the person reads, or `-` when that is not known.
    pub language: String,
    pub instruction: String,
}

/// What the Language line means to the worker. The worker's own default for `-` is spelled out
/// because an unfilled line is otherwise read as "no rule".
const LANGUAGE_GLOSS: &str = "(the language of everything you write for the person: gates and their titles, questions, the plan, self-review and verification notes, card notes and your reports. Commit messages, PR bodies, issues and code follow the repository's conventions. `-`: the language the person uses with you, or the one your agent is set to)";

const OPENING: &str = "You are the one working in this worktree. You are not the hub (the side that hands tasks out).\n\n";

const REPORT_TO_TASK: &str =
    "**the user at this tab**. Do not send results to the hub — the hub only hands work out,
  and has nowhere to pass on what it receives. You send the hub only these two things on your own:
  (a) a bug **outside this task**, through the `adj-report` procedure, and (b) a request to clean up
  once the work is done.
  (Answering when the hub asks with `[question {id}]` is neither of these, and is fine to do)";

const TASK_OVERRIDES: &str = "Important overrides. If you remember the hub's procedure, the following take precedence:

1. Do not use `isolation: worktree`. It digs yet another worktree.
   Work directly in the current cwd.
2. Do not use `EnterWorktree`. You are already inside.
3. Neither `git -C <worktree_path>` nor an absolute worktree path is needed. Plain `git` and relative
   paths are fine.
4. The reviewers' (sub-agents' / codex's) working directory is the current cwd too.
5. Do not try to hand the implementation back to the hub. The hub only hands work out. Do everything
   up to the final report yourself, and **give that report to the user at this tab** (\"Report to\"
   above; do not send it on to the hub).
6. If you find \"a bug unrelated to the current task\" while working, **do not fix it yourself**.
   A diff with unrelated fixes mixed in can be neither reviewed nor reverted.
   Hand it to the hub following the `adj skill adj-report` procedure (or `adjutant_skill`
   `name=adj-report`), and go back to your task.

**First run `adj skill adj-worker` (or `adjutant_skill` `name=adj-worker`) and follow the procedure it
prints.** Everything is written there. Do not read only the brief and go your own way.

Start by reading the task's body and comments.
";

const SESSION_BODY: &str = "This session has no task. Do what the instruction says, with the person in this tab. The gates, the
card and the PR steps of `adj-worker` apply only once a task is linked to this session: until then
there is no record to open a gate for or to move to \"in review\". Check `adjutant_outbox` after each
step and before you answer the person — a message headed `[linked {id}]` means the person has linked
this session to a task, and `adj skill adj-worker` says what to do from there.

Important overrides. If you remember the hub's procedure, the following take precedence:

1. Do not use `isolation: worktree` or `EnterWorktree`. You are already inside; work in the current cwd.
2. Neither `git -C <worktree_path>` nor an absolute worktree path is needed.
3. If you find \"a bug unrelated to what you were asked\" while working, **do not fix it yourself**.
   Hand it to the hub following the `adj skill adj-report` procedure, and go back to what you were doing.

## Instruction

";

/// One line of the brief. The value of a line, and the lines under it, are indented: a
/// title or a handover note is somebody's text, and one that begins `- Done when:` must not
/// read as a second line of that name.
fn line(out: &mut String, label: &str, value: &str, gloss: Option<&str>) {
    // A blank value is `-`, the spelling for "none": a line ending in a space reads as
    // missing, and the worker falls back to asking.
    let value = if value.trim().is_empty() { "-" } else { value };
    let mut lines = value.lines();
    let first = lines.next().unwrap_or("");
    let rest: Vec<&str> = lines.collect();
    if rest.is_empty() {
        out.push_str(&format!("- {label}: {first}\n"));
    } else {
        out.push_str(&format!("- {label}:\n"));
        for text in std::iter::once(first).chain(rest) {
            indented(out, text);
        }
    }
    if let Some(gloss) = gloss {
        for text in gloss.lines() {
            indented(out, text);
        }
    }
}

/// A line of text under a label. Blank lines stay blank, so no line is only spaces.
fn indented(out: &mut String, text: &str) {
    if !text.trim().is_empty() {
        out.push_str("  ");
        out.push_str(text);
    }
    out.push('\n');
}

/// A line whose text is fixed, written as it is: the continuation lines are part of it.
fn fixed(out: &mut String, label: &str, text: &str) {
    out.push_str(&format!("- {label}: {text}\n"));
}

/// Text that must stay on one line: a title or a key, where a newline would start a line of
/// the brief that nobody wrote.
fn one_line(text: &str) -> String {
    text.split(['\r', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn render_task(brief: &TaskBrief) -> String {
    let mut out = String::from(OPENING);

    out.push_str(&format!(
        "- {}: {} \"{}\" ({})\n",
        label::TASK,
        one_line(&brief.key),
        one_line(&brief.title),
        brief.tracker.as_str()
    ));
    match brief.url.as_deref().filter(|url| !url.trim().is_empty()) {
        Some(url) => indented(&mut out, &one_line(url)),
        None if brief.request.trim().is_empty() => indented(&mut out, "-"),
        None => {
            for text in brief.request.trim_end().lines() {
                indented(&mut out, text);
            }
        }
    }
    fixed(
        &mut out,
        label::WORKSPACE,
        &format!(
            "the current cwd is that worktree (branch {})",
            one_line(&brief.branch)
        ),
    );
    line(&mut out, label::BASE_BRANCH, &brief.base, None);
    line(
        &mut out,
        label::PARENT_TASK,
        &brief.parent,
        Some("(the URL of the task this one belongs under; `-` if there is none)"),
    );
    line(
        &mut out,
        label::TASK_RECORD,
        &brief.record,
        Some(&format!(
            "(the id of the board's card. Put it in `task` when opening a gate and it is tied to\nthe card on the board. Once you open a PR, move the card on with\n`adj task update --id {} --status pr --pr <URL>`)",
            one_line(&brief.record)
        )),
    );
    line(
        &mut out,
        label::DONE_WHEN,
        &brief.done_when,
        Some(
            "(what the hub was asked by the user, as it is. Unless it is \"up to a PR\", do not open a PR. For\n\"investigation only\", do not implement, commit, or file or update an issue)",
        ),
    );
    line(
        &mut out,
        label::STOP_AT,
        &brief.stop_at,
        Some(
            "(which gates wait on a person. `plan` is plan approval only, `diff` the plan and the diff review,\n`all` the plan, the diff review and verification)",
        ),
    );
    line(
        &mut out,
        label::HANDOVER_NOTE,
        &brief.handover,
        Some(
            "(the handover note or extra instructions given from the dashboard; `-` if there are none)",
        ),
    );
    line(
        &mut out,
        label::COPILOT_REVIEW,
        &brief.copilot_review,
        Some(
            "(whether to ask Copilot for a review after opening the PR. `ask` asks every time, `always` requests\nit without asking, `never` does not request it and does not ask)",
        ),
    );
    line(
        &mut out,
        label::LANGUAGE,
        &one_line(&brief.language),
        Some(LANGUAGE_GLOSS),
    );
    fixed(&mut out, label::REPORT_TO, REPORT_TO_TASK);
    let verify: Vec<&String> = brief
        .verify
        .iter()
        .filter(|command| !command.trim().is_empty())
        .collect();
    if verify.is_empty() {
        fixed(&mut out, label::VERIFY_COMMANDS, "-");
    } else {
        out.push_str(&format!("- {}:\n", label::VERIFY_COMMANDS));
        for command in verify {
            // A command that spans lines keeps them, under its bullet.
            let mut lines = command.trim_end().lines();
            out.push_str(&format!(
                "  - {}\n",
                lines.next().unwrap_or("").trim_start()
            ));
            for continuation in lines {
                if continuation.trim().is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("    {continuation}\n"));
                }
            }
        }
        indented(&mut out, "(the config's `verify`, one command per bullet)");
    }
    out.push('\n');
    out.push_str(TASK_OVERRIDES);
    out
}

pub fn render_session(brief: &SessionBrief) -> String {
    let mut out = String::from(OPENING);
    fixed(&mut out, label::TASK, "-");
    fixed(
        &mut out,
        label::WORKSPACE,
        &format!(
            "the current cwd is that worktree (branch {})",
            one_line(&brief.branch)
        ),
    );
    line(&mut out, label::BASE_BRANCH, &brief.base, None);
    fixed(&mut out, label::PARENT_TASK, "-");
    fixed(&mut out, label::TASK_RECORD, "-");
    fixed(&mut out, label::DONE_WHEN, "as the instruction says");
    line(
        &mut out,
        label::LANGUAGE,
        &one_line(&brief.language),
        Some(LANGUAGE_GLOSS),
    );
    fixed(&mut out, label::REPORT_TO, "**the user at this tab**.");
    out.push('\n');
    out.push_str(SESSION_BODY);
    // Last and as it was written: the one place text is not indented, because nothing
    // follows it that it could be mistaken for.
    out.push_str(brief.instruction.trim_end());
    out.push('\n');
    out
}

/// The tracker and the key of the task an issue URL names, from the config alone.
///
/// A GitHub issue has no key of its own: the key is the repository's `issueKeys` entry and
/// the issue number, which is how the hub has always named it. `None` when the URL is not one
/// of the shapes below, or a GitHub repository has no entry, and the caller says so rather
/// than guess: the worker reads the ticket with the tool the tracker names.
pub fn tracker_and_key(
    url: &str,
    sources: &[Value],
    issue_keys: &Map<String, Value>,
) -> Option<(Tracker, String)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let mut parts = rest.split('/').filter(|part| !part.is_empty());
    let host = parts.next()?;
    let path: Vec<&str> = parts.collect();

    if host.eq_ignore_ascii_case("linear.app")
        && let [_team, "issue", key, ..] = path.as_slice()
        && is_key(key)
    {
        return Some((Tracker::Linear, key.to_string()));
    }
    if let Some(at) = path.iter().position(|part| *part == "browse")
        && let Some(key) = path.get(at + 1)
        && is_key(key)
    {
        return Some((Tracker::Jira, key.to_string()));
    }
    if let [owner, repo, "issues", number, ..] = path.as_slice()
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
    {
        let repo = format!("{owner}/{repo}");
        let prefix = issue_keys
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&repo))
            .and_then(|(_, key)| key.as_str())?;
        let key = format!("{prefix}-{number}");
        if !is_key(&key) {
            return None;
        }
        let type_of = |source: &Value| {
            source
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let tracker = if sources.iter().any(|source| {
            type_of(source).as_deref() == Some("github")
                && source
                    .get("issueRepo")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(&repo))
        }) {
            Tracker::Github
        } else if sources
            .iter()
            .any(|source| type_of(source).as_deref() == Some("github-project"))
        {
            Tracker::GithubProject
        } else {
            Tracker::Github
        };
        return Some((tracker, key));
    }
    None
}

/// `[A-Za-z][A-Za-z0-9_-]*-[0-9]+`, the shape every tracker's key has. `issueKeys` values are
/// not restricted, so the prefix may hold a `-` (`MY-ABC-12`): the number is what follows the last one.
pub fn is_key(text: &str) -> bool {
    let Some((prefix, number)) = text.rsplit_once('-') else {
        return false;
    };
    let mut chars = prefix.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        && !prefix.ends_with('-')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> TaskBrief {
        TaskBrief {
            key: "WID-12".to_string(),
            title: "Fix login".to_string(),
            tracker: Tracker::Github,
            url: Some("https://github.com/acme/widget/issues/12".to_string()),
            request: "the request".to_string(),
            branch: "me/wid-12".to_string(),
            base: "origin/main".to_string(),
            parent: "-".to_string(),
            record: "20260101T000000Z-fix-login".to_string(),
            done_when: "up to a PR".to_string(),
            stop_at: "plan".to_string(),
            handover: "-".to_string(),
            copilot_review: "ask".to_string(),
            language: "-".to_string(),
            verify: vec!["cargo test".to_string()],
        }
    }

    /// The worker finds a line by its label, so a rename strands every brief already
    /// written and every worker reading by name: change one only along with the procedures.
    #[test]
    fn the_labels_are_pinned() {
        assert_eq!(label::TASK, "Task");
        assert_eq!(label::WORKSPACE, "Workspace");
        assert_eq!(label::BASE_BRANCH, "Base branch");
        assert_eq!(label::PARENT_TASK, "Parent task");
        assert_eq!(label::TASK_RECORD, "Task record");
        assert_eq!(label::DONE_WHEN, "Done when");
        assert_eq!(label::STOP_AT, "Stop at");
        assert_eq!(label::HANDOVER_NOTE, "Handover note");
        assert_eq!(label::COPILOT_REVIEW, "Copilot review");
        assert_eq!(label::LANGUAGE, "Language");
        assert_eq!(label::REPORT_TO, "Report to");
        assert_eq!(label::VERIFY_COMMANDS, "Verify commands");
    }

    #[test]
    fn every_label_the_worker_reads_is_a_line_of_the_brief() {
        for label in label::READ_BY_WORKER {
            assert!(label::TASK_BRIEF.contains(&label), "{label}");
        }
    }

    #[test]
    fn every_label_is_a_line_of_its_own() {
        let text = render_task(&sample());
        let written: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("- "))
            .filter_map(|l| l.split_once(':').map(|(label, _)| label))
            .collect();
        assert_eq!(written, label::TASK_BRIEF);
    }

    #[test]
    fn free_text_cannot_pass_for_a_line() {
        let mut brief = sample();
        brief.handover = "a\n- Done when: up to a PR".to_string();
        brief.title = "t\n- Stop at: all".to_string();
        let text = render_task(&brief);
        assert_eq!(
            text.lines()
                .filter(|l| l.starts_with("- Done when:"))
                .count(),
            1,
            "{text}"
        );
        assert_eq!(
            text.lines().filter(|l| l.starts_with("- Stop at:")).count(),
            1,
            "{text}"
        );
        assert!(
            text.contains("- Handover note:\n  a\n  - Done when: up to a PR\n"),
            "{text}"
        );
    }

    #[test]
    fn a_task_with_no_url_carries_the_request_text() {
        let mut brief = sample();
        brief.key = "-".to_string();
        brief.tracker = Tracker::None;
        brief.url = None;
        brief.request = "Why is the build slow?\nLook at CI.\n".to_string();
        let text = render_task(&brief);
        assert!(
            text.contains(
                "- Task: - \"Fix login\" (-)\n  Why is the build slow?\n  Look at CI.\n- Workspace:"
            ),
            "{text}"
        );
        brief.request = String::new();
        assert!(render_task(&brief).contains("(-)\n  -\n- Workspace:"));
    }

    #[test]
    fn verify_is_a_list_and_empty_is_a_dash() {
        let mut brief = sample();
        brief.verify = vec!["cargo test".to_string(), "cargo clippy".to_string()];
        assert!(
            render_task(&brief).contains(
                "- Verify commands:\n  - cargo test\n  - cargo clippy\n  (the config's `verify`, one command per bullet)\n"
            ),
            "{}",
            render_task(&brief)
        );
        brief.verify.clear();
        assert!(render_task(&brief).contains("- Verify commands: -\n"));
    }

    #[test]
    fn a_session_brief_has_no_task_and_ends_with_the_instruction_verbatim() {
        let text = render_session(&SessionBrief {
            branch: "me/scratch".to_string(),
            base: "origin/main".to_string(),
            language: "ja".to_string(),
            instruction: "do this\n## not a header\n\n".to_string(),
        });
        assert!(text.contains("- Task: -\n"), "{text}");
        assert!(text.contains("- Task record: -\n"), "{text}");
        assert!(text.contains("- Parent task: -\n"), "{text}");
        assert!(text.contains("- Language: ja\n"), "{text}");
        assert!(text.contains("(branch me/scratch)"), "{text}");
        assert!(
            text.ends_with("## Instruction\n\ndo this\n## not a header\n"),
            "{text}"
        );
    }

    #[test]
    fn a_language_line_is_one_line_and_blank_is_a_dash() {
        let mut brief = sample();
        brief.language = "ja\n- Done when: never".to_string();
        let text = render_task(&brief);
        assert!(
            text.contains("- Language: ja - Done when: never\n"),
            "{text}"
        );
        assert_eq!(
            text.lines()
                .filter(|l| l.starts_with("- Done when:"))
                .count(),
            1,
            "{text}"
        );
        brief.language = "  ".to_string();
        assert!(render_task(&brief).contains("- Language: -\n"));
    }

    #[test]
    fn a_multi_line_verify_command_keeps_its_lines_under_its_bullet() {
        let mut brief = sample();
        brief.verify = vec!["cargo test \\\n--all".to_string(), "  ".to_string()];
        assert!(
            render_task(&brief).contains(
                "- Verify commands:\n  - cargo test \\\n    --all\n  (the config's `verify`"
            ),
            "{}",
            render_task(&brief)
        );
    }

    #[test]
    fn a_blank_value_is_a_dash_and_a_blank_url_is_no_url() {
        let mut brief = sample();
        brief.parent = " ".to_string();
        brief.handover = String::new();
        brief.url = Some(String::new());
        brief.request = "Why is it slow?".to_string();
        let text = render_task(&brief);
        assert!(text.contains("- Parent task: -\n"), "{text}");
        assert!(text.contains("- Handover note: -\n"), "{text}");
        assert!(text.contains("\n  Why is it slow?\n- Workspace:"), "{text}");
        assert!(text.lines().all(|l| l == l.trim_end()), "{text}");
    }

    #[test]
    fn a_tracker_is_read_back_from_its_name_and_anything_else_is_refused() {
        for tracker in [
            Tracker::Github,
            Tracker::GithubProject,
            Tracker::Jira,
            Tracker::Linear,
            Tracker::None,
        ] {
            assert_eq!(Tracker::parse(tracker.as_str()), Ok(tracker));
        }
        assert_eq!(
            Tracker::parse("bogus"),
            Err("no such tracker: bogus (github, github-project, jira, linear or -)".to_string())
        );
        assert!(Tracker::parse(" jira").is_err());
        assert!(Tracker::parse("GitHub").is_err());
    }

    #[test]
    fn tracker_and_key_reads_each_tracker() {
        let sources = vec![
            json!({"type": "github", "issueRepo": "acme/widget"}),
            json!({"type": "github-project", "projectOwner": "acme"}),
        ];
        let keys = json!({"acme/widget": "WID", "acme/other": "WEB"});
        let keys = keys.as_object().unwrap();
        let read = |url: &str| tracker_and_key(url, &sources, keys);
        let pair = |tracker: Tracker, key: &str| Some((tracker, key.to_string()));

        assert_eq!(
            read("https://github.com/acme/widget/issues/12"),
            pair(Tracker::Github, "WID-12")
        );
        assert_eq!(
            read("https://github.com/Acme/Other/issues/7#issuecomment-1"),
            pair(Tracker::GithubProject, "WEB-7")
        );
        assert_eq!(
            read("https://example.atlassian.net/browse/ABC-345?focusedId=1"),
            pair(Tracker::Jira, "ABC-345")
        );
        assert_eq!(
            read("https://linear.app/acme/issue/XYZ-9/fix-the-thing"),
            pair(Tracker::Linear, "XYZ-9")
        );
        assert_eq!(read("https://github.com/acme/unknown/issues/3"), None);
        assert_eq!(read("not a url"), None);
        assert_eq!(read("https://example.com/somewhere"), None);
        assert_eq!(read("https://example.atlassian.net/browse/lower"), None);
        assert_eq!(
            read("https://example.atlassian.net/browse/MY_ABC-12"),
            pair(Tracker::Jira, "MY_ABC-12")
        );
        assert!(!is_key("_X-1"));
        assert!(is_key("MY-ABC-12"));
        for bad in ["MY--12", "-1", "A-", "A-1x", "A", "A-1; id"] {
            assert!(!is_key(bad), "{bad}");
        }
    }
}
