//! The clap definitions of the command line.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "adjutant",
    version,
    about = "Agent task orchestrator: hands work out to workers, takes their reports back in"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// This repository's hub session name — the address a report is sent to
    HubName {
        /// owner/name (default: derived from origin)
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// Also print the slug, main checkout and where the name came from
        #[arg(long)]
        json: bool,
    },
    /// The resolved configuration for this repository, as JSON
    Config {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
    },
    /// Messages waiting for this repository's hub
    Pending {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// Print only the inbox directory, creating it if absent
        #[arg(long)]
        path: bool,
        /// Max entries to list
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
        /// Print one message in full
        #[arg(long, value_name = "NAME")]
        read: Option<String>,
        /// File one message away once it has been dealt with
        #[arg(long, value_name = "NAME")]
        ack: Option<String>,
    },
    /// Hand a message to this repository's hub (body from --body or stdin)
    Send {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// Who is sending: your session or worktree name
        #[arg(long)]
        from: Option<String>,
        /// report | question | answer | ack | done | needs-user | request | next | gate
        #[arg(long, default_value = "report")]
        kind: String,
        /// One line stating the conclusion
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        body: Option<String>,
        /// Say nothing on success
        #[arg(long)]
        quiet: bool,
        /// Wake the receiver (default: automatic based on message kind and sender)
        #[arg(long, overrides_with = "no_wake")]
        wake: bool,
        /// Deliver without waking the receiver
        #[arg(long, overrides_with = "wake")]
        no_wake: bool,
    },
    /// Open a terminal tab and run a command there
    Spawn {
        #[arg(long)]
        repo: Option<String>,
        /// Directory the new tab starts in
        #[arg(long)]
        cwd: String,
        /// Tab title
        #[arg(long, default_value = "")]
        title: String,
        /// Print the command or script instead of running it
        #[arg(long)]
        dry_run: bool,
        /// -- followed by the command to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Open a tab and start a worker agent on a worktree; exit 3 if maxWorkers are already running
    Work {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
        #[arg(long, default_value = "")]
        title: String,
        /// Take the tab's title from this task record instead of --title
        #[arg(long, value_name = "ID", conflicts_with_all = ["title", "resume"])]
        task: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, then the one agentEnv names; a
        /// worktree's own record is not read here)
        #[arg(long)]
        hub: Option<String>,
        /// What the worker is told on startup (default: read .claude/task-brief.md; with
        /// --resume, check the outbox and carry on)
        #[arg(long)]
        prompt: Option<String>,
        /// Reopen the worker session saved in this worktree instead of starting a new one
        #[arg(long)]
        resume: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Start the worker agent in this tab (what `work` opens a tab to run)
    Worker {
        #[arg(long)]
        repo: Option<String>,
        /// Default with --resume: the worktree this is run from
        #[arg(long, required_unless_present = "resume")]
        worktree: Option<String>,
        /// Default with --resume: the title the worker was started with
        #[arg(long)]
        title: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, then the one agentEnv names; a
        /// worktree's own record is not read here. With --resume: the hub that dispatched the
        /// saved session)
        #[arg(long)]
        hub: Option<String>,
        /// What the worker is told on startup (default: read .claude/task-brief.md; with
        /// --resume, check the outbox and carry on)
        #[arg(long)]
        prompt: Option<String>,
        /// Reopen the worker session saved in this worktree instead of starting a new one
        #[arg(long)]
        resume: bool,
        /// The task record ID when one is linked
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Leave a message for the worker in a worktree (body from --body or stdin)
    Tell {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// One line stating the point. `[question …]` is what makes a worker answer
        #[arg(long)]
        subject: String,
        #[arg(long)]
        body: Option<String>,
        /// Who is speaking (default: this repository's hub name)
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        quiet: bool,
        /// Wake the receiver (default: automatic based on subject)
        #[arg(long, overrides_with = "no_wake")]
        wake: bool,
        /// Deliver without waking the receiver
        #[arg(long, overrides_with = "wake")]
        no_wake: bool,
    },
    /// Print one of the procedures: adj-hub | adj-worker | adj-report
    Skill {
        name: String,
        /// Free text substituted into the procedure where it asks for it
        #[arg(long, default_value = "")]
        arguments: String,
        /// Target agent format: claude | agy | generic (default: auto-detect)
        #[arg(long, value_parser = ["claude", "claude-code", "agy", "antigravity", "generic", "codex"])]
        agent: Option<String>,
    },
    /// What the hub has left for the worker in this worktree
    Outbox {
        /// Default: the current directory
        #[arg(long)]
        worktree: Option<String>,
        /// Everything in it has been dealt with
        #[arg(long)]
        clear: bool,
    },
    /// Bring this repository's running hub (or, with --worktree, a worker) to the front; exit 1 if it is not running
    Focus {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// Bring the worker in this worktree to the front instead of the hub
        #[arg(long, conflicts_with = "hub")]
        worktree: Option<String>,
        /// Say nothing, use the exit code
        #[arg(long)]
        quiet: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Say which step the worker in this worktree is in, or show it
    Phase {
        /// plan | implement | self-review | verify | pr | pr-bots | review | report
        #[arg(long, value_name = "PHASE")]
        set: Option<String>,
        /// Default: the worktree this is run from
        #[arg(long)]
        worktree: Option<String>,
    },
    /// Which engine reads the diff in this self-review round: reviewEngine, then Claude's rate limits
    ReviewEngine {
        #[arg(long)]
        repo: Option<String>,
        /// Print the decision, the window that tripped and the message as JSON
        #[arg(long)]
        json: bool,
    },
    /// Close the tab the worker in a worktree is sitting in; exit 1 if it is still there
    Close {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
        /// Say nothing, use the exit code
        #[arg(long)]
        quiet: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Start this repository's hub, in the main checkout, once
    Hub {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// Which hub of the repository (default: $ADJUTANT_HUB, then the one agentEnv names; a
        /// worktree's own record is not read here)
        #[arg(long)]
        hub: Option<String>,
        /// Open a tab and start it there, instead of becoming it in this one
        #[arg(long)]
        tab: bool,
        /// Reopen this hub's saved session instead of starting a new one (without either flag,
        /// a session that ended within hubAutoResumeHours is resumed)
        #[arg(long, conflicts_with = "new")]
        resume: bool,
        /// Start a new session even when the last one ended within hubAutoResumeHours
        #[arg(long)]
        new: bool,
        /// Skip the dashboard collection this hub runs at startup (overrides startupDashboard)
        #[arg(long, conflicts_with = "dashboard")]
        no_dashboard: bool,
        /// Collect the dashboard at startup even where startupDashboard is off
        #[arg(long)]
        dashboard: bool,
        /// Extra arguments appended to the agent command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },
    /// Clear this repository's hub record
    HubStop {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
    },
    /// Close a parent-task hub whose workers are all gone: clear its record and take it off the board
    HubClose {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
    },
    /// Open a worktree in the configured editor
    Ide {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Name the tab this process is running in (the hub names its own)
    Title {
        #[arg(long)]
        repo: Option<String>,
        /// `-` reads it from stdin
        #[arg(long)]
        title: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Tell the human something happened
    Notify {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value = "adjutant")]
        title: String,
        #[arg(long)]
        message: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Where a task's worktree goes, and what branch it sits on, with no convention tool
    WorktreePath {
        #[arg(long)]
        repo: Option<String>,
        /// A branch you already know: prints the path only
        #[arg(long)]
        branch: Option<String>,
        /// A task name (app-1234): prints branch, path and main as JSON
        #[arg(long)]
        name: Option<String>,
        /// Branch owner (default: $USER)
        #[arg(long)]
        user: Option<String>,
        /// The task source's branchPattern, if it has one
        #[arg(long)]
        pattern: Option<String>,
        /// With --name: take the first of name, name-2, name-3… whose path and branch are free
        #[arg(long, requires = "name", conflicts_with = "branch")]
        unique: bool,
    },
    /// Run the stdio MCP server
    Mcp,
    /// Register the MCP server with an agent
    InstallMcp {
        /// claude-code | agy | json
        #[arg(long, default_value = "claude-code")]
        target: String,
    },
    /// Serve this repository's dashboard on a local port
    Serve {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// 0 picks a free port, for a second dashboard on the same machine
        #[arg(long, default_value_t = crate::board::DEFAULT_PORT)]
        port: u16,
        /// Print the URL without opening a browser
        #[arg(long)]
        no_open: bool,
    },
    /// The resident server: every repository's board, whether or not a hub is running
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },
    /// Tasks handed to this repository's hub
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Gates: what an agent has put up for a person to look at
    Gate {
        #[command(subcommand)]
        action: GateAction,
    },
    /// Hand a task's approved plan to Jules, and ask how it is going
    Jules {
        #[command(subcommand)]
        action: JulesAction,
    },
    /// Remove the MCP server registration
    UninstallMcp {
        /// claude-code | agy
        #[arg(long, default_value = "claude-code")]
        target: String,
    },
    /// Tmux backend helpers: pane inspection, window spawning, wake, close, and focus
    Tmux {
        #[command(subcommand)]
        action: TmuxAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum ServerAction {
    /// Start the resident server, detached unless --foreground
    Start {
        /// 0 picks a free port; a taken port falls back to a free one
        #[arg(long, default_value_t = crate::board::DEFAULT_PORT)]
        port: u16,
        /// Stay in this process, for a service manager
        #[arg(long)]
        foreground: bool,
        /// Print the URL without opening a browser
        #[arg(long)]
        no_open: bool,
    },
    /// Stop the resident server. Hubs keep running
    Stop,
    /// Stop the resident server and start it again on the same port. Hubs and workers keep running
    Restart {
        /// Port for the new server; the old one's port when omitted
        #[arg(long)]
        port: Option<u16>,
        /// Open the board in a browser once it is up
        #[arg(long)]
        open: bool,
    },
    /// Whether the resident server is running, and which boards it serves (exit 1 when not)
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum GateAction {
    /// Hand the ball over. The payload is JSON, on stdin unless --file says otherwise
    Open {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        /// Read the payload from here instead of stdin
        #[arg(long)]
        file: Option<String>,
        /// Take the gate's body from this file as it is (the payload must not have one)
        #[arg(long)]
        body_file: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// What is waiting for a person
    List {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One gate's payload, as JSON
    Show {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
    },
    /// Hand the ball back: deliver the decision to that worktree's outbox, or to the hub's inbox for a gate the hub opened
    Answer {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
        /// approve | changes | reject | choice | ack | ask | answer
        #[arg(long)]
        decision: String,
        /// Which of the gate's choices, for `--decision choice`
        #[arg(long)]
        choice: Option<String>,
        #[arg(long)]
        comment: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Archive a gate without delivering an answer to the worker (e.g. dealt with in tab)
    Close {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
        /// Reason or note for closing
        #[arg(long)]
        comment: Option<String>,
        /// The person answered in the worker's terminal
        #[arg(long)]
        terminal: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum JulesAction {
    /// Start a Jules session for a task and write its id onto the record
    Start {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        /// The task being handed over
        #[arg(long)]
        id: String,
        /// File holding the prompt (the design Jules implements), or - for stdin
        #[arg(long)]
        prompt_file: String,
        /// Branch Jules starts from (default: the task's own base)
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// How a session is doing: its state, its page and its pull request
    Show {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        /// The task whose session to show
        #[arg(long, conflicts_with = "session", required_unless_present = "session")]
        id: Option<String>,
        /// A session id, when there is no task for it
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// The review bots' comments on the task's pull request that could be passed on to Jules
    Findings {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Pass review comments on to Jules, as one comment on the pull request in your name
    Relay {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
        /// A comment id from `adj jules findings` (repeat for more)
        #[arg(
            long = "comment",
            required_unless_present = "plan_file",
            conflicts_with = "plan_file"
        )]
        comments: Vec<String>,
        /// A relay plan as JSON: {"note": …, "findings": [{"id": …, "note": …}]} — a note for each
        /// comment, as the hub prepares them
        #[arg(long)]
        plan_file: Option<String>,
        /// Something to say to Jules above them (- reads stdin)
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum TmuxAction {
    /// Inspect tmux panes matching a pid or tty
    Pane {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        socket: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        tty: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Wake a process running in a tmux window
    Wake {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        socket: Option<String>,
        #[arg(long)]
        pid: u32,
        #[arg(long)]
        line: Option<String>,
        /// The agent in the pane, whose screen is read before typing: claude | agy | generic
        /// (default: generic, which types without looking)
        #[arg(long, value_parser = ["claude", "claude-code", "agy", "antigravity", "generic", "codex"])]
        agent: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Focus a tmux window running a process
    Focus {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        socket: Option<String>,
        #[arg(long)]
        pid: u32,
        #[arg(long)]
        dry_run: bool,
    },
    /// Close a tmux window running a process
    Close {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        socket: Option<String>,
        #[arg(long)]
        pid: u32,
        #[arg(long)]
        dry_run: bool,
    },
    /// Spawn a command in a detached tmux window
    Spawn {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        socket: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        title: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum TaskAction {
    /// Write a task down, and hand it over if asked
    Add {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        /// Short title for the card (inferred from body when omitted)
        #[arg(long)]
        title: Option<String>,
        /// What is being asked for (may come on stdin as `-`)
        #[arg(long)]
        body: Option<String>,
        /// start | file-and-start | investigate | tell-worker
        #[arg(long, default_value = "start")]
        kind: String,
        /// report-only | verify | pr | review
        #[arg(long, default_value = "pr")]
        done_when: String,
        /// Which gates wait on a person: plan | diff (plan and diff) | all (plan, diff and verify)
        #[arg(long, default_value = "plan")]
        stop_at: String,
        /// Who implements once the plan is approved: worker | jules
        #[arg(long, default_value = "worker")]
        executor: String,
        #[arg(long)]
        issue_url: Option<String>,
        /// What this one dispatch should branch from
        #[arg(long)]
        base: Option<String>,
        /// The parent task's URL
        #[arg(long)]
        parent: Option<String>,
        /// Needed only when there is no issue to take a name from
        #[arg(long)]
        worktree_name: Option<String>,
        /// Have the hub confirm before it starts
        #[arg(long)]
        ask_first: bool,
        /// Hand it to the hub now, rather than leaving it in the backlog
        #[arg(long)]
        queue: bool,
        /// Queue it with this worktree already made, sending the hub nothing — for the hub
        /// itself, writing down work it is about to start or that is waiting for a slot
        #[arg(long, value_name = "WORKTREE", conflicts_with = "queue")]
        waiting_in: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// What this hub has been handed
    List {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        /// backlog | queued | dispatched | pr | done | cancelled
        #[arg(long)]
        status: Option<String>,
        /// Only the tasks being worked on in this worktree
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One task's record, as JSON
    Show {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
    },
    /// The queued task a free worker slot should take next, and the ones waiting on a
    /// confirmation nobody has been asked for
    Next {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Move every task whose pull request was merged to done, and say what was left alone
    Refresh {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Read the task's issue again and keep its title and body on the record
    FetchIssue {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Move a task on. This is how the hub reports back what it did with one
    Update {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        id: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        order: Option<u32>,
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        issue: Option<String>,
        #[arg(long)]
        pr: Option<String>,
        /// The branch it is cut from, as `git worktree add` takes it ('' clears it)
        #[arg(long)]
        base: Option<String>,
        /// The Jules session implementing it ('' clears it)
        #[arg(long)]
        jules_session: Option<String>,
        /// Who implements once the plan is approved: worker | jules
        #[arg(long)]
        executor: Option<String>,
        /// Why it could not be taken, when that is the answer ('' clears it, - reads stdin)
        #[arg(long)]
        note: Option<String>,
        /// Handover instruction for the agent when queued ('' clears it, - reads stdin)
        #[arg(long)]
        instruction: Option<String>,
        /// Whether the hub may start it without asking first
        #[arg(long, value_name = "true|false")]
        auto_start: Option<bool>,
        /// Queue it without putting a request in the hub's inbox — for the hub itself
        #[arg(long)]
        no_hand_over: bool,
        #[arg(long)]
        json: bool,
    },
    /// Write the worker's .claude/task-brief.md from the task record and the config (no --id: a
    /// task-less session's brief)
    Brief {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        /// The task the worker is for. Without it, a session with no task
        #[arg(long)]
        id: Option<String>,
        /// The worktree the worker works in
        #[arg(long, value_name = "PATH")]
        worktree: String,
        /// What the branch was cut from, as `git worktree add` took it (- when not known)
        #[arg(long, value_name = "COMMIT-ISH")]
        base: String,
        /// The tracker's key, when it cannot be read from the issue URL
        #[arg(long, requires = "id")]
        key: Option<String>,
        /// github | github-project | jira | linear, when it cannot be read from the issue URL
        #[arg(long, requires = "id")]
        tracker: Option<String>,
        /// The parent task's URL (the record's own, when this is left out)
        #[arg(long, requires = "id")]
        parent: Option<String>,
        /// What the person asked of a session with no task (- reads stdin)
        #[arg(long, required_unless_present = "id", conflicts_with = "id")]
        instruction: Option<String>,
        /// Write here instead of {worktree}/.claude/task-brief.md
        #[arg(long, value_name = "PATH")]
        out: Option<String>,
        #[arg(long)]
        json: bool,
    },
}
