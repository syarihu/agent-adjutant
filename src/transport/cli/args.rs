//! The clap definitions of the command line.

use clap::{Args, Parser, Subcommand};

use crate::lifecycle::hub::HubStart;

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
    HubName(HubNameArgs),
    /// The resolved configuration for this repository, as JSON
    Config(ConfigArgs),
    /// Messages waiting for this repository's hub
    Pending(PendingArgs),
    /// Hand a message to this repository's hub (body from --body or stdin)
    Send(SendArgs),
    /// Open a terminal tab and run a command there
    Spawn(SpawnArgs),
    /// Open a tab and start a worker agent on a worktree; exit 3 if maxWorkers are already running
    Work(WorkArgs),
    /// Start the worker agent in this tab (what `work` opens a tab to run)
    Worker(WorkerArgs),
    /// Leave a message for the worker in a worktree (body from --body or stdin)
    Tell(TellArgs),
    /// Print one of the procedures: adj-hub | adj-worker | adj-report
    Skill(SkillArgs),
    /// What the hub has left for the worker in this worktree
    Outbox(OutboxArgs),
    /// Bring this repository's running hub (or, with --worktree, a worker) to the front; exit 1 if it is not running
    Focus(FocusArgs),
    /// Say which step the worker in this worktree is in, or show it
    Phase(PhaseArgs),
    /// Which engine reads the diff in this self-review round: reviewEngine, then Claude's rate limits
    ReviewEngine(ReviewEngineArgs),
    /// What each agent session's hooks last said: running, waiting, done, sub-agents, its last message
    AgentSessions(AgentSessionsArgs),
    /// Add adjutant's hooks to an agent's user settings, so sessions adjutant did not start report too
    Setup(SetupArgs),
    /// Record an agent's hook event from stdin (what the agent's settings run)
    #[command(hide = true)]
    Hook(HookArgs),
    /// Close the tab the worker in a worktree is sitting in; exit 1 if it is still there
    Close(CloseArgs),
    /// Start this repository's hub, in the main checkout, once
    Hub(HubArgs),
    /// Clear this repository's hub record
    HubStop(HubStopArgs),
    /// Close a parent-task hub whose workers are all gone: clear its record and take it off the board
    HubClose(HubCloseArgs),
    /// Open a worktree in the configured editor
    Ide(IdeArgs),
    /// Name the tab this process is running in (the hub names its own)
    Title(TitleArgs),
    /// Tell the human something happened
    Notify(NotifyArgs),
    /// Where a task's worktree goes, and what branch it sits on, with no convention tool
    WorktreePath(WorktreePathArgs),
    /// Run the stdio MCP server
    Mcp,
    /// Register the MCP server with an agent
    InstallMcp(InstallMcpArgs),
    /// Serve this repository's dashboard on a local port
    Serve(ServeArgs),
    /// The resident server: every repository's board, whether or not a hub is running
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },
    /// Tasks handed to this repository's hub
    Task {
        // Boxed: `task update` has the most options of any command, and unboxed it sets the
        // size of every `Commands` value.
        #[command(subcommand)]
        action: Box<TaskAction>,
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
    UninstallMcp(UninstallMcpArgs),
    /// Tmux backend helpers: pane inspection, window spawning, wake, close, and focus
    Tmux {
        #[command(subcommand)]
        action: TmuxAction,
    },
}

#[derive(Args)]
pub(crate) struct HubNameArgs {
    /// owner/name (default: derived from origin)
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// Also print the slug, main checkout and where the name came from
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct ConfigArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
}

#[derive(Args)]
pub(crate) struct PendingArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// Print only the inbox directory, creating it if absent
    #[arg(long)]
    pub(crate) path: bool,
    /// Max entries to list
    #[arg(long, default_value_t = 20)]
    pub(crate) limit: usize,
    #[arg(long)]
    pub(crate) json: bool,
    /// Print one message in full, which marks it as looked at
    #[arg(long, value_name = "NAME")]
    pub(crate) read: Option<String>,
    /// File one message away once it has been dealt with
    #[arg(long, value_name = "NAME")]
    pub(crate) ack: Option<String>,
}

#[derive(Args)]
pub(crate) struct SendArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// Who is sending: your session or worktree name
    #[arg(long)]
    pub(crate) from: Option<String>,
    /// report | question | answer | ack | done | needs-user | request | next | gate
    #[arg(long, default_value = "report")]
    pub(crate) kind: String,
    /// One line stating the conclusion
    #[arg(long)]
    pub(crate) subject: Option<String>,
    #[arg(long)]
    pub(crate) body: Option<String>,
    /// Say nothing on success
    #[arg(long)]
    pub(crate) quiet: bool,
    /// Wake the receiver (default: automatic based on message kind and sender)
    #[arg(long, overrides_with = "no_wake")]
    pub(crate) wake: bool,
    /// Deliver without waking the receiver
    #[arg(long, overrides_with = "wake")]
    pub(crate) no_wake: bool,
}

impl SendArgs {
    pub(crate) fn wake(&self) -> Option<bool> {
        if self.wake {
            Some(true)
        } else if self.no_wake {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Args)]
pub(crate) struct SpawnArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Directory the new tab starts in
    #[arg(long)]
    pub(crate) cwd: String,
    /// Tab title
    #[arg(long, default_value = "")]
    pub(crate) title: String,
    /// Print the command or script instead of running it
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// -- followed by the command to run
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub(crate) command: Vec<String>,
}

impl SpawnArgs {
    /// The command to run, without the `--` that may come before it.
    pub(crate) fn command(&self) -> Vec<String> {
        strip_separator(&self.command)
    }
}

#[derive(Args)]
pub(crate) struct WorkArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) worktree: String,
    #[arg(long, default_value = "")]
    pub(crate) title: String,
    /// Take the tab's title from this task record instead of --title
    #[arg(long, value_name = "ID", conflicts_with_all = ["title", "resume"])]
    pub(crate) task: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, then the one agentEnv names; a
    /// worktree's own record is not read here)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// What the worker is told on startup (default: read .claude/task-brief.md; with
    /// --resume, check the outbox and carry on)
    #[arg(long)]
    pub(crate) prompt: Option<String>,
    /// Reopen the worker session saved in this worktree instead of starting a new one
    #[arg(long)]
    pub(crate) resume: bool,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct WorkerArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Default with --resume: the worktree this is run from
    #[arg(long, required_unless_present = "resume")]
    pub(crate) worktree: Option<String>,
    /// Default with --resume: the title the worker was started with
    #[arg(long)]
    pub(crate) title: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, then the one agentEnv names; a
    /// worktree's own record is not read here. With --resume: the hub that dispatched the
    /// saved session)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// What the worker is told on startup (default: read .claude/task-brief.md; with
    /// --resume, check the outbox and carry on)
    #[arg(long)]
    pub(crate) prompt: Option<String>,
    /// Reopen the worker session saved in this worktree instead of starting a new one
    #[arg(long)]
    pub(crate) resume: bool,
    /// The task record ID when one is linked
    #[arg(long)]
    pub(crate) task: Option<String>,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct TellArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) worktree: String,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// One line stating the point. `[question …]` is what makes a worker answer
    #[arg(long)]
    pub(crate) subject: String,
    #[arg(long)]
    pub(crate) body: Option<String>,
    /// Who is speaking (default: this repository's hub name)
    #[arg(long)]
    pub(crate) from: Option<String>,
    #[arg(long)]
    pub(crate) quiet: bool,
    /// Wake the receiver (default: automatic based on subject)
    #[arg(long, overrides_with = "no_wake")]
    pub(crate) wake: bool,
    /// Deliver without waking the receiver
    #[arg(long, overrides_with = "wake")]
    pub(crate) no_wake: bool,
}

impl TellArgs {
    pub(crate) fn wake(&self) -> Option<bool> {
        if self.wake {
            Some(true)
        } else if self.no_wake {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Args)]
pub(crate) struct SkillArgs {
    pub(crate) name: String,
    /// Free text substituted into the procedure where it asks for it
    #[arg(long, default_value = "")]
    pub(crate) arguments: String,
    /// Target agent format: claude | agy | generic (default: auto-detect)
    #[arg(long, value_parser = ["claude", "claude-code", "agy", "antigravity", "generic", "codex"])]
    pub(crate) agent: Option<String>,
}

#[derive(Args)]
pub(crate) struct OutboxArgs {
    /// Default: the current directory
    #[arg(long)]
    pub(crate) worktree: Option<String>,
    /// Everything in it has been dealt with
    #[arg(long)]
    pub(crate) clear: bool,
}

#[derive(Args)]
pub(crate) struct FocusArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// Bring the worker in this worktree to the front instead of the hub
    #[arg(long, conflicts_with = "hub")]
    pub(crate) worktree: Option<String>,
    /// Say nothing, use the exit code
    #[arg(long)]
    pub(crate) quiet: bool,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct PhaseArgs {
    /// plan | implement | self-review | verify | pr | pr-bots | review | report
    #[arg(long, value_name = "PHASE")]
    pub(crate) set: Option<String>,
    /// Default: the worktree this is run from
    #[arg(long)]
    pub(crate) worktree: Option<String>,
}

#[derive(Args)]
pub(crate) struct ReviewEngineArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Print the decision, the window that tripped and the message as JSON
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct AgentSessionsArgs {
    /// Print the rows as a JSON array, as they are stored
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct SetupArgs {
    /// Which agent's settings: claude, codex
    #[arg(value_parser = ["claude", "codex"])]
    pub(crate) agent: String,
    /// Take out only the entries `adj setup` added (any `adj` path), and nothing else
    #[arg(long)]
    pub(crate) remove: bool,
}

#[derive(Args)]
pub(crate) struct HookArgs {
    /// Which agent's payload this is: claude, codex
    pub(crate) agent: String,
    /// Marks an entry `adj setup` wrote into a global settings file; it changes nothing here
    #[arg(long, hide = true)]
    pub(crate) global: bool,
    /// Read Claude Code's status line input instead of a hook payload: record the model, context use and rate limits on the session's existing row, print nothing (claude only)
    #[arg(long)]
    pub(crate) status_line: bool,
}

#[derive(Args)]
pub(crate) struct CloseArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) worktree: String,
    /// Say nothing, use the exit code
    #[arg(long)]
    pub(crate) quiet: bool,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct HubArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Which hub of the repository (default: $ADJUTANT_HUB, then the one agentEnv names; a
    /// worktree's own record is not read here)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// Open a tab and start it there, instead of becoming it in this one
    #[arg(long)]
    pub(crate) tab: bool,
    /// Reopen this hub's saved session instead of starting a new one (without either flag,
    /// a session that ended within hubAutoResumeHours is resumed)
    #[arg(long, conflicts_with = "new")]
    pub(crate) resume: bool,
    /// Start a new session even when the last one ended within hubAutoResumeHours
    #[arg(long)]
    pub(crate) new: bool,
    /// Skip the dashboard collection this hub runs at startup (overrides startupDashboard)
    #[arg(long, conflicts_with = "dashboard")]
    pub(crate) no_dashboard: bool,
    /// Collect the dashboard at startup even where startupDashboard is off
    #[arg(long)]
    pub(crate) dashboard: bool,
    /// Extra arguments appended to the agent command
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub(crate) extra: Vec<String>,
}

impl HubArgs {
    pub(crate) fn start(&self) -> HubStart {
        match (self.resume, self.new) {
            (true, _) => HubStart::Resume,
            (_, true) => HubStart::New,
            _ => HubStart::Auto,
        }
    }

    /// Two flags, three answers. `None` is "nobody said", and it has to stay distinct
    /// from both: it is what leaves the configured value standing, and what keeps a
    /// plain `adj hub` printing the command line it has always printed. The standing
    /// `startupDashboard` then answers on its own.
    pub(crate) fn dashboard(&self) -> Option<bool> {
        match (self.no_dashboard, self.dashboard) {
            (true, _) => Some(false),
            (_, true) => Some(true),
            _ => None,
        }
    }

    /// The extra arguments for the agent command, without the `--` that may come before them.
    pub(crate) fn extra(&self) -> Vec<String> {
        strip_separator(&self.extra)
    }
}

#[derive(Args)]
pub(crate) struct HubStopArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
}

#[derive(Args)]
pub(crate) struct HubCloseArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
}

#[derive(Args)]
pub(crate) struct IdeArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) worktree: String,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct TitleArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// `-` reads it from stdin
    #[arg(long)]
    pub(crate) title: String,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct NotifyArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long, default_value = "adjutant")]
    pub(crate) title: String,
    #[arg(long)]
    pub(crate) message: String,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct WorktreePathArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// A branch you already know: prints the path only
    #[arg(long)]
    pub(crate) branch: Option<String>,
    /// A task name (app-1234): prints branch, path and main as JSON
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// Branch owner (default: $USER)
    #[arg(long)]
    pub(crate) user: Option<String>,
    /// The task source's branchPattern, if it has one
    #[arg(long)]
    pub(crate) pattern: Option<String>,
    /// With --name: take the first of name, name-2, name-3… whose path and branch are free
    #[arg(long, requires = "name", conflicts_with = "branch")]
    pub(crate) unique: bool,
}

#[derive(Args)]
pub(crate) struct ServeArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// 0 picks a free port, for a second dashboard on the same machine
    #[arg(long, default_value_t = crate::board::DEFAULT_PORT)]
    pub(crate) port: u16,
    /// Print the URL without opening a browser
    #[arg(long)]
    pub(crate) no_open: bool,
}

#[derive(Args)]
pub(crate) struct InstallMcpArgs {
    /// claude-code | agy | json
    #[arg(long, default_value = "claude-code")]
    pub(crate) target: String,
}

#[derive(Args)]
pub(crate) struct UninstallMcpArgs {
    /// claude-code | agy
    #[arg(long, default_value = "claude-code")]
    pub(crate) target: String,
}

#[derive(Subcommand)]
pub(crate) enum ServerAction {
    /// Start the resident server, detached unless --foreground
    Start(ServerStartArgs),
    /// Stop the resident server. Hubs keep running
    Stop,
    /// Stop the resident server and start it again on the same port. Hubs and workers keep running
    Restart(ServerRestartArgs),
    /// Whether the resident server is running, and which boards it serves (exit 1 when not)
    Status(ServerStatusArgs),
}

#[derive(Args)]
pub(crate) struct ServerStartArgs {
    /// 0 picks a free port; a taken port falls back to a free one
    #[arg(long, default_value_t = crate::board::DEFAULT_PORT)]
    pub(crate) port: u16,
    /// Stay in this process, for a service manager
    #[arg(long)]
    pub(crate) foreground: bool,
    /// Print the URL without opening a browser
    #[arg(long)]
    pub(crate) no_open: bool,
}

#[derive(Args)]
pub(crate) struct ServerRestartArgs {
    /// Port for the new server; the old one's port when omitted
    #[arg(long)]
    pub(crate) port: Option<u16>,
    /// Open the board in a browser once it is up
    #[arg(long)]
    pub(crate) open: bool,
}

#[derive(Args)]
pub(crate) struct ServerStatusArgs {
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Subcommand)]
pub(crate) enum GateAction {
    /// Hand the ball over. The payload is JSON, on stdin unless --file says otherwise
    Open(GateOpenArgs),
    /// What is waiting for a person
    List(GateListArgs),
    /// One gate's payload, as JSON
    Show(GateShowArgs),
    /// Hand the ball back: deliver the decision to that worktree's outbox, or to the hub's inbox for a gate the hub opened
    Answer(GateAnswerArgs),
    /// Archive a gate without delivering an answer to the worker (e.g. dealt with in tab)
    Close(GateCloseArgs),
}

#[derive(Args)]
pub(crate) struct GateOpenArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// Read the payload from here instead of stdin
    #[arg(long)]
    pub(crate) file: Option<String>,
    /// Take the gate's body from this file as it is (the payload must not have one)
    #[arg(long)]
    pub(crate) body_file: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct GateListArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct GateShowArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
}

#[derive(Args)]
pub(crate) struct GateAnswerArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    /// approve | changes | reject | choice | ack | ask | answer
    #[arg(long)]
    pub(crate) decision: String,
    /// Which of the gate's choices, for `--decision choice`
    #[arg(long)]
    pub(crate) choice: Option<String>,
    #[arg(long)]
    pub(crate) comment: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct GateCloseArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    /// Reason or note for closing
    #[arg(long)]
    pub(crate) comment: Option<String>,
    /// The person answered in the worker's terminal
    #[arg(long)]
    pub(crate) terminal: bool,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Subcommand)]
pub(crate) enum JulesAction {
    /// Start a Jules session for a task and write its id onto the record
    Start(JulesStartArgs),
    /// How a session is doing: its state, its page and its pull request
    Show(JulesShowArgs),
    /// The review bots' comments on the task's pull request that could be passed on to Jules
    Findings(JulesFindingsArgs),
    /// Pass review comments on to Jules, as one comment on the pull request in your name
    Relay(JulesRelayArgs),
}

#[derive(Args)]
pub(crate) struct JulesStartArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// The task being handed over
    #[arg(long)]
    pub(crate) id: String,
    /// File holding the prompt (the design Jules implements), or - for stdin
    #[arg(long)]
    pub(crate) prompt_file: String,
    /// Branch Jules starts from (default: the task's own base)
    #[arg(long)]
    pub(crate) base: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct JulesShowArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// The task whose session to show
    #[arg(long, conflicts_with = "session", required_unless_present = "session")]
    pub(crate) id: Option<String>,
    /// A session id, when there is no task for it
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct JulesFindingsArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct JulesRelayArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    /// A comment id from `adj jules findings` (repeat for more)
    #[arg(
        long = "comment",
        required_unless_present = "plan_file",
        conflicts_with = "plan_file"
    )]
    pub(crate) comments: Vec<String>,
    /// A relay plan as JSON: {"note": …, "findings": [{"id": …, "note": …}]} — a note for each
    /// comment, as the hub prepares them
    #[arg(long)]
    pub(crate) plan_file: Option<String>,
    /// Something to say to Jules above them (- reads stdin)
    #[arg(long)]
    pub(crate) note: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum TmuxAction {
    /// Inspect tmux panes matching a pid or tty
    Pane(TmuxPaneArgs),
    /// Wake a process running in a tmux window
    Wake(TmuxWakeArgs),
    /// Focus a tmux window running a process
    Focus(TmuxFocusArgs),
    /// Close a tmux window running a process
    Close(TmuxCloseArgs),
    /// Spawn a command in a detached tmux window
    Spawn(TmuxSpawnArgs),
}

#[derive(Args)]
pub(crate) struct TmuxPaneArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) socket: Option<String>,
    #[arg(long)]
    pub(crate) pid: Option<u32>,
    #[arg(long)]
    pub(crate) tty: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TmuxWakeArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) socket: Option<String>,
    #[arg(long)]
    pub(crate) pid: u32,
    #[arg(long)]
    pub(crate) line: Option<String>,
    /// The agent in the pane, whose screen is read before typing: claude | agy | generic
    /// (default: generic, which types without looking)
    #[arg(long, value_parser = ["claude", "claude-code", "agy", "antigravity", "generic", "codex"])]
    pub(crate) agent: Option<String>,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct TmuxFocusArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) socket: Option<String>,
    #[arg(long)]
    pub(crate) pid: u32,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct TmuxCloseArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) socket: Option<String>,
    #[arg(long)]
    pub(crate) pid: u32,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct TmuxSpawnArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) socket: Option<String>,
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[arg(long)]
    pub(crate) cwd: Option<String>,
    #[arg(long)]
    pub(crate) title: String,
    #[arg(long)]
    pub(crate) dry_run: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub(crate) command: Vec<String>,
}

impl TmuxSpawnArgs {
    /// The command to run, without the `--` that may come before it.
    pub(crate) fn command(&self) -> Vec<String> {
        strip_separator(&self.command)
    }
}

/// clap keeps the `--` in a trailing var-arg, and the caller meant it as a separator rather
/// than as the first word of the command.
pub(crate) fn strip_separator(args: &[String]) -> Vec<String> {
    match args.split_first() {
        Some((first, rest)) if first == "--" => rest.to_vec(),
        _ => args.to_vec(),
    }
}

#[derive(Subcommand)]
pub(crate) enum TaskAction {
    /// Write a task down, and hand it over if asked
    Add(TaskAddArgs),
    /// What this hub has been handed
    List(TaskListArgs),
    /// One task's record, as JSON
    Show(TaskShowArgs),
    /// The queued task a free worker slot should take next, and the ones waiting on a
    /// confirmation nobody has been asked for
    Next(TaskNextArgs),
    /// Move every task whose pull request was merged to done, and say what was left alone
    Refresh(TaskRefreshArgs),
    /// Read the task's issue again and keep its title and body on the record
    FetchIssue(TaskFetchIssueArgs),
    /// Move a task on. This is how the hub reports back what it did with one
    Update(TaskUpdateArgs),
    /// Write the worker's .claude/task-brief.md from the task record and the config (no --id: a
    /// task-less session's brief)
    Brief(TaskBriefArgs),
    /// Park a task you are waiting on on purpose (someone else's answer, the right time to merge)
    Park(TaskParkArgs),
    /// Take a task's park back
    Unpark(TaskUnparkArgs),
}

#[derive(Args)]
pub(crate) struct TaskAddArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// Short title for the card (inferred from body when omitted)
    #[arg(long)]
    pub(crate) title: Option<String>,
    /// What is being asked for (may come on stdin as `-`)
    #[arg(long)]
    pub(crate) body: Option<String>,
    /// start | file-and-start | investigate | tell-worker
    #[arg(long, default_value = "start")]
    pub(crate) kind: String,
    /// report-only | verify | pr | review
    #[arg(long, default_value = "pr")]
    pub(crate) done_when: String,
    /// Which gates wait on a person: plan | diff (plan and diff) | all (plan, diff and verify)
    #[arg(long, default_value = "plan")]
    pub(crate) stop_at: String,
    /// Who implements once the plan is approved: worker | jules
    #[arg(long, default_value = "worker")]
    pub(crate) executor: String,
    #[arg(long)]
    pub(crate) issue_url: Option<String>,
    /// What this one dispatch should branch from
    #[arg(long)]
    pub(crate) base: Option<String>,
    /// The parent task's URL
    #[arg(long)]
    pub(crate) parent: Option<String>,
    /// Needed only when there is no issue to take a name from
    #[arg(long)]
    pub(crate) worktree_name: Option<String>,
    /// Have the hub confirm before it starts
    #[arg(long)]
    pub(crate) ask_first: bool,
    /// The body is all the person said: the hub reads it, fills in the rest and asks on a gate
    /// before anything starts (what the board's form sends)
    #[arg(long)]
    pub(crate) needs_reading: bool,
    /// Hand it to the hub now, rather than leaving it in the backlog
    #[arg(long)]
    pub(crate) queue: bool,
    /// Queue it with this worktree already made, sending the hub nothing — for the hub
    /// itself, writing down work it is about to start or that is waiting for a slot
    #[arg(long, value_name = "WORKTREE", conflicts_with = "queue")]
    pub(crate) waiting_in: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskListArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// backlog | queued | dispatched | pr | done | cancelled
    #[arg(long)]
    pub(crate) status: Option<String>,
    /// Only the tasks being worked on in this worktree
    #[arg(long)]
    pub(crate) worktree: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskShowArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
}

#[derive(Args)]
pub(crate) struct TaskNextArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskRefreshArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskFetchIssueArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    /// Make the issue's title the task's title, not only when the task has none yet
    #[arg(long)]
    pub(crate) take_title: bool,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskUpdateArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) status: Option<String>,
    #[arg(long)]
    pub(crate) order: Option<u32>,
    #[arg(long)]
    pub(crate) worktree: Option<String>,
    #[arg(long)]
    pub(crate) issue: Option<String>,
    #[arg(long)]
    pub(crate) pr: Option<String>,
    /// The branch it is cut from, as `git worktree add` takes it ('' clears it)
    #[arg(long)]
    pub(crate) base: Option<String>,
    /// The parent task's URL, or a key the hub turns into one ('' clears it)
    #[arg(long)]
    pub(crate) parent: Option<String>,
    /// The Jules session implementing it ('' clears it)
    #[arg(long)]
    pub(crate) jules_session: Option<String>,
    /// Who implements once the plan is approved: worker | jules
    #[arg(long)]
    pub(crate) executor: Option<String>,
    /// What the task is: start | file-and-start | investigate
    #[arg(long)]
    pub(crate) kind: Option<String>,
    /// What counts as done: report-only | verify | pr | review
    #[arg(long)]
    pub(crate) done_when: Option<String>,
    /// Which gates wait on a person: plan | diff (plan and diff) | all (plan, diff and verify)
    #[arg(long)]
    pub(crate) stop_at: Option<String>,
    /// The issue the task works on, as a URL ('' clears it)
    #[arg(long)]
    pub(crate) issue_url: Option<String>,
    /// The name its worktree and branch take, for a task with no issue to name them ('' clears it)
    #[arg(long)]
    pub(crate) worktree_name: Option<String>,
    /// The card's title (- reads stdin; it cannot be blank)
    #[arg(long)]
    pub(crate) title: Option<String>,
    /// The person confirmed the hub's reading on the gate; refused unless the record can start
    #[arg(long)]
    pub(crate) read: bool,
    /// Why it could not be taken, when that is the answer ('' clears it, - reads stdin)
    #[arg(long)]
    pub(crate) note: Option<String>,
    /// Handover instruction for the agent when queued ('' clears it, - reads stdin)
    #[arg(long)]
    pub(crate) instruction: Option<String>,
    /// Whether the hub may start it without asking first
    #[arg(long, value_name = "true|false")]
    pub(crate) auto_start: Option<bool>,
    /// Queue it without putting a request in the hub's inbox — for the hub itself
    #[arg(long)]
    pub(crate) no_hand_over: bool,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskParkArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    /// What it waits on: pdm | design | review | merge-timing | other
    #[arg(long, value_name = "REASON")]
    pub(crate) reason: String,
    /// Said in words (required for other; - reads stdin)
    #[arg(long)]
    pub(crate) text: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskUnparkArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    #[arg(long)]
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct TaskBriefArgs {
    #[arg(long)]
    pub(crate) repo: Option<String>,
    #[arg(long)]
    pub(crate) hub: Option<String>,
    /// The task the worker is for. Without it, a session with no task
    #[arg(long)]
    pub(crate) id: Option<String>,
    /// The worktree the worker works in
    #[arg(long, value_name = "PATH")]
    pub(crate) worktree: String,
    /// What the branch was cut from, as `git worktree add` took it (- when not known)
    #[arg(long, value_name = "COMMIT-ISH")]
    pub(crate) base: String,
    /// The tracker's key, when it cannot be read from the issue URL
    #[arg(long, requires = "id")]
    pub(crate) key: Option<String>,
    /// github | github-project | jira | linear, when it cannot be read from the issue URL
    #[arg(long, requires = "id")]
    pub(crate) tracker: Option<String>,
    /// The parent task's URL (the record's own, when this is left out)
    #[arg(long, requires = "id")]
    pub(crate) parent: Option<String>,
    /// The language the person reads, used when the config has no `language`
    #[arg(long)]
    pub(crate) language: Option<String>,
    /// What the person asked of a session with no task (- reads stdin)
    #[arg(long, required_unless_present = "id", conflicts_with = "id")]
    pub(crate) instruction: Option<String>,
    /// Write here instead of {worktree}/.claude/task-brief.md
    #[arg(long, value_name = "PATH")]
    pub(crate) out: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
}
