use serde_json::{Map, Value, json};

/// Falls back to `defaults`, then to these. `ide` has no built-in on purpose: guessing an
/// editor puts the user in the wrong one, and the prompts know how to ask.
pub fn builtin_defaults() -> Map<String, Value> {
    json!({
        "baseBranch": "auto",
        "reviewBots": [],
        "reviewEffort": "high",
        "reviewEngine": "auto",
        "selfReviewRounds": 5,
        "draftPr": true,
        // Whether the worker asks before requesting a Copilot review on the PR it opens.
        // Separate from `reviewBots`, which names the reviews to wait for, not whether to
        // ask for one.
        "copilotReview": "ask",
        "issueKeys": {},
        "verify": [],
        "postCreate": [],
        "onWorktreeRemove": [],
        // Named procedures the prompts hand off to — PR style, comment style, issue
        // creation. Absent means "skip that step", which is what makes the prompts usable
        // on a machine that has none of them.
        "skills": {},
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

/// Keys that describe *a task source*, not the repo. In the flat shorthand they sit beside
/// a top-level `taskSource`; normalising lifts them into `taskSources[0]`.
pub const SOURCE_KEYS: [&str; 8] = [
    "issueRepo",
    "projectOwner",
    "projectNumber",
    "projectFields",
    "branchPattern",
    "worktreeName",
    "linear",
    "jira",
];

/// Trackers number their issues independently, so WID-233 and XYZ-233 both exist and a
/// key-less worktree name collides.
pub const DEFAULT_WORKTREE_NAME: &str = "{issuekey-lowercase}-{issue}";

/// Keys that configure *the machine*, not the work. They are resolved into `Settings` and
/// kept out of the per-repo config so there is only ever one copy of each.
pub(super) const SETTING_KEYS: [&str; 18] = [
    "terminal",
    "notification",
    "agentRunner",
    "agentEnv",
    "hubRunner",
    // What `--resume` runs instead of the two above. Beside them for the reason they are here:
    // which agent a session is, and how it is reopened, is the machine's business.
    "agentResumeRunner",
    "hubResumeRunner",
    "wake",
    "hubWake",
    "workerWake",
    "worktreePattern",
    // Which editor to open a worktree in is a property of the machine, like the rest of
    // these. Left out of this list it survived into the per-repo config as well as into
    // `Settings`, so `adj config` answered with it twice and the two could disagree.
    "ide",
    // Whether the hub spends its first block dispatching the dashboard collector. Machine
    // level because it is about how somebody wants their hub to come up, not about the work
    // the repository holds — and it is here rather than in the prompt because the prompt is
    // the same text on every machine.
    "startupDashboard",
    // Whether a hub's MCP server serves that hub's board. About the machine — whether a
    // local port may be opened, and whether somebody runs `adj serve` themselves — like the
    // one above.
    "hubServe",
    // How recently a hub has to have ended for a plain `adj hub` to bring it back rather than
    // start a new one. About how somebody works, like the one above.
    "hubAutoResumeHours",
    // How many workers may run at once. A property of the machine — its memory, its CPU, the
    // agent's rate limit — and not of any one repository's work.
    "maxWorkers",
    // How long a worker may stay in one phase before the board flags it. About how somebody
    // works, like the two above it.
    "stuckAfterMinutes",
    // How to get the Jules API key. The key belongs to a person's account and sits in their
    // keychain, not to any one repository, so there is one answer per machine.
    "julesKey",
];

/// How long a worker may sit in one phase before the board calls it stuck, when nothing is
/// configured. Long enough for a review round or a slow build; short enough that a worker
/// that stopped at lunch is noticed in the afternoon.
pub const DEFAULT_STUCK_AFTER_MINUTES: f64 = 120.0;

/// The window a plain `adj hub` resumes in, when nothing is configured.
///
/// Long enough to cover an update, a crash, lunch; short enough that the first hub of the
/// morning starts clean. The hub is meant to be one a day, and a conversation that carries on
/// forever carries every task it ever dispatched along with it.
pub const DEFAULT_HUB_AUTO_RESUME_HOURS: f64 = 3.0;
