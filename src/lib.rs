//! adjutant — hands work out to workers, takes their reports back in.
//!
//! One binary, two modes. The subcommands are for shells, hooks and launchers — the places
//! that run *before* an agent exists and so cannot ask one for anything. `adjutant mcp` is
//! the same machinery served to an agent that is already running.
//!
//! Shipped as a library with two thin binaries over it (`adjutant` and its short name
//! `adj`), so the same code is one build rather than two, and the integration tests can
//! drive it directly.

mod cmd;
mod config;
mod gate;
mod http;
mod ide;
mod mcp;
mod messaging;
mod notify;
mod prompts;
mod repo;
mod runner;
mod task;
mod template;
mod terminal;

/// Scaffolding the tests share. Not a layer — nothing outside `#[cfg(test)]` may reach it,
/// which is why `check-layering.sh` lets any module name it.
#[cfg(test)]
pub(crate) mod testing {
    /// `ADJUTANT_CONFIG`, `ADJUTANT_STATE_DIR` and `ADJUTANT_HUB` are process-global and
    /// the harness runs tests in parallel threads, so a sandbox has to be exclusive or two
    /// tests read each other's config and each other's inboxes.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub struct Sandbox {
        _dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Sandbox {
        /// Point both the config and the state directory at a tempdir. A test that reads
        /// the real ones answers about the developer's own machine, and a test that writes
        /// to them delivers its fixtures to a hub that is actually running.
        pub fn new(config_json: &str) -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("config.json");
            std::fs::write(&config, config_json).unwrap();
            unsafe {
                std::env::set_var("ADJUTANT_STATE_DIR", dir.path().join("state"));
                std::env::set_var("ADJUTANT_CONFIG", &config);
                // `cargo test` run from inside a hub's own session inherits this, and every
                // test that asserts on an address would then be answering about that hub.
                std::env::remove_var("ADJUTANT_HUB");
                // The same trap one variable over, and a quieter one: a hub started with
                // `--no-dashboard` exports this, so a test run in that hub's tab would see
                // `startupDashboard` resolve to `false` no matter what its fixture said —
                // and the settings tests would fail for a reason nothing in them mentions.
                std::env::remove_var(crate::config::STARTUP_DASHBOARD_ENV);
            }
            Sandbox {
                _dir: dir,
                _guard: guard,
            }
        }

        /// For tests that only care about where messages go.
        pub fn empty() -> Self {
            Sandbox::new("{\"repos\": {}}")
        }
    }
}

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "adjutant",
    version,
    about = "Agent task orchestrator: hands work out to workers, takes their reports back in"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
        /// report | question | answer | ack | done | needs-user
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
    /// Open a tab and start a worker agent on a worktree
    Work {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
        #[arg(long, default_value = "")]
        title: String,
        /// Which hub of the repository (default: $ADJUTANT_HUB; a worktree's own record is not read here)
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
        /// Which hub of the repository (default: $ADJUTANT_HUB; a worktree's own record is not
        /// read here. With --resume: the hub that dispatched the saved session)
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
    /// Leave a message for the worker in a worktree (body from --body or stdin)
    Tell {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// One line stating the point. `[質問 …]` is what makes a worker answer
        #[arg(long)]
        subject: String,
        #[arg(long)]
        body: Option<String>,
        /// Who is speaking (default: this repository's hub name)
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        quiet: bool,
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
    /// Bring this repository's running hub to the front; exit 1 if it is not running
    Focus {
        #[arg(long)]
        repo: Option<String>,
        /// Which hub of the repository (default: $ADJUTANT_HUB, or the one that dispatched this worktree)
        #[arg(long)]
        hub: Option<String>,
        /// Say nothing, use the exit code
        #[arg(long)]
        quiet: bool,
        #[arg(long)]
        dry_run: bool,
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
        /// Which hub of the repository (default: $ADJUTANT_HUB; a worktree's own record is not read here)
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
        #[arg(long, default_value_t = cmd::DEFAULT_PORT)]
        port: u16,
        /// Print the URL without opening a browser
        #[arg(long)]
        no_open: bool,
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
    /// Remove the MCP server registration
    UninstallMcp {
        /// claude-code | agy
        #[arg(long, default_value = "claude-code")]
        target: String,
    },
}

#[derive(Subcommand)]
enum GateAction {
    /// Hand the ball over. The payload is JSON, on stdin unless --file says otherwise
    Open {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        hub: Option<String>,
        /// Read the payload from here instead of stdin
        #[arg(long)]
        file: Option<String>,
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
    /// Hand the ball back: deliver the decision to that worktree's outbox
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
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TaskAction {
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
        /// Why it could not be taken, when that is the answer
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

/// Parse this process's arguments and run the subcommand. Exits; never returns.
pub fn run() -> ! {
    let cli = Cli::parse();
    let result: Result<i32, String> = match &cli.command {
        Commands::HubName { repo, hub, json } => {
            cmd::hub_name(repo.as_deref(), hub.as_deref(), *json).map(|_| 0)
        }
        Commands::Config { repo, hub } => {
            cmd::show_config(repo.as_deref(), hub.as_deref()).map(|_| 0)
        }
        Commands::Pending {
            repo,
            hub,
            path,
            limit,
            json,
            read,
            ack,
        } => cmd::pending(&cmd::PendingArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            path_only: *path,
            limit: *limit,
            as_json: *json,
            read: read.as_deref(),
            ack: ack.as_deref(),
        })
        .map(|_| 0),
        Commands::Send {
            repo,
            hub,
            from,
            kind,
            subject,
            body,
            quiet,
        } => cmd::send(&cmd::SendArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            from: from.as_deref(),
            kind,
            subject: subject.as_deref(),
            body: body.as_deref(),
            quiet: *quiet,
        })
        .map(|_| 0),
        Commands::Spawn {
            repo,
            cwd,
            title,
            dry_run,
            command,
        } => cmd::spawn(
            repo.as_deref(),
            cwd,
            title,
            &strip_separator(command),
            *dry_run,
        )
        .map(|_| 0),
        Commands::Work {
            repo,
            hub,
            worktree,
            title,
            prompt,
            resume,
            dry_run,
        } => cmd::work(
            repo.as_deref(),
            hub.as_deref(),
            worktree,
            title,
            prompt.as_deref(),
            *resume,
            *dry_run,
        )
        .map(|_| 0),
        // Exit 1 when the hub is not running, so a shell can branch on it without parsing
        // anything this prints.
        Commands::Worker {
            repo,
            hub,
            worktree,
            title,
            prompt,
            resume,
            dry_run,
        } => cmd::worker(
            repo.as_deref(),
            hub.as_deref(),
            worktree.as_deref(),
            title.as_deref(),
            prompt.as_deref(),
            *resume,
            *dry_run,
        )
        .map(|_| 0),
        Commands::Tell {
            repo,
            hub,
            worktree,
            subject,
            body,
            from,
            quiet,
        } => cmd::tell(
            repo.as_deref(),
            hub.as_deref(),
            worktree,
            subject,
            body.as_deref(),
            from.as_deref(),
            *quiet,
        )
        .map(|_| 0),
        Commands::Serve {
            repo,
            hub,
            port,
            no_open,
        } => cmd::serve(repo.as_deref(), hub.as_deref(), *port, !*no_open).map(|_| 0),
        Commands::Gate { action } => run_gate(action).map(|_| 0),
        Commands::Task { action } => run_task(action).map(|_| 0),
        Commands::Skill {
            name,
            arguments,
            agent,
        } => cmd::skill(name, arguments, agent.as_deref()).map(|_| 0),
        Commands::Outbox { worktree, clear } => cmd::outbox(worktree.as_deref(), *clear).map(|_| 0),
        Commands::Focus {
            repo,
            hub,
            quiet,
            dry_run,
        } => cmd::focus(repo.as_deref(), hub.as_deref(), *quiet, *dry_run)
            .map(|found| i32::from(!found)),
        Commands::Close {
            repo,
            worktree,
            quiet,
            dry_run,
        } => {
            cmd::close(repo.as_deref(), worktree, *quiet, *dry_run).map(|closed| i32::from(!closed))
        }
        Commands::Hub {
            repo,
            hub,
            dry_run,
            tab,
            resume,
            new,
            no_dashboard,
            dashboard,
            extra,
        } => cmd::hub(
            repo.as_deref(),
            hub.as_deref(),
            &strip_separator(extra),
            *tab,
            match (*resume, *new) {
                (true, _) => cmd::HubStart::Resume,
                (_, true) => cmd::HubStart::New,
                _ => cmd::HubStart::Auto,
            },
            // Two flags, three answers. `None` is "nobody said", and it has to stay distinct
            // from both: it is what leaves the configured value standing, and what keeps a
            // plain `adj hub` printing the command line it has always printed.
            match (*no_dashboard, *dashboard) {
                (true, _) => Some(false),
                (_, true) => Some(true),
                _ => None,
            },
            *dry_run,
        )
        .map(|_| 0),
        Commands::HubStop { repo, hub } => {
            cmd::hub_stop(repo.as_deref(), hub.as_deref()).map(|_| 0)
        }
        Commands::Ide {
            repo,
            worktree,
            dry_run,
        } => cmd::open_ide(repo.as_deref(), worktree, *dry_run).map(|_| 0),
        Commands::Title {
            repo,
            title,
            dry_run,
        } => cmd::set_title(repo.as_deref(), title, *dry_run).map(|_| 0),
        Commands::Notify {
            repo,
            title,
            message,
            dry_run,
        } => cmd::notify_user(repo.as_deref(), title, message, *dry_run).map(|_| 0),
        Commands::WorktreePath {
            repo,
            branch,
            name,
            user,
            pattern,
        } => cmd::worktree_path(&cmd::WorktreeArgs {
            repo: repo.as_deref(),
            branch: branch.as_deref(),
            name: name.as_deref(),
            user: user.as_deref(),
            pattern: pattern.as_deref(),
        })
        .map(|_| 0),
        Commands::Mcp => mcp::run_server().map(|_| 0).map_err(|e| e.to_string()),
        Commands::InstallMcp { target } => mcp::install(target).map(|_| 0),
        Commands::UninstallMcp { target } => mcp::uninstall(target).map(|_| 0),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("adjutant: {message}");
            std::process::exit(1);
        }
    }
}

/// clap keeps the `--` in a trailing var-arg, and the caller meant it as a separator rather
/// than as the first word of the command.
fn strip_separator(args: &[String]) -> Vec<String> {
    match args.split_first() {
        Some((first, rest)) if first == "--" => rest.to_vec(),
        _ => args.to_vec(),
    }
}

/// The `adj task` verbs. Split out of `run` because the four of them are a subcommand of a
/// subcommand, and inlining that match would bury the rest of the dispatch.
fn run_task(action: &TaskAction) -> Result<(), String> {
    match action {
        TaskAction::Add {
            repo,
            hub,
            title,
            body,
            kind,
            done_when,
            issue_url,
            base,
            parent,
            worktree_name,
            ask_first,
            queue,
            json,
        } => cmd::task_add(&cmd::AddArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            title: title.as_deref(),
            body: body.as_deref(),
            kind,
            done_when,
            issue_url: issue_url.as_deref(),
            base: base.as_deref(),
            parent: parent.as_deref(),
            worktree_name: worktree_name.as_deref(),
            ask_first: *ask_first,
            queue: *queue,
            json: *json,
        }),
        TaskAction::List {
            repo,
            hub,
            status,
            json,
        } => cmd::task_list(repo.as_deref(), hub.as_deref(), status.as_deref(), *json),
        TaskAction::Show { repo, hub, id } => cmd::task_show(repo.as_deref(), hub.as_deref(), id),
        TaskAction::Update {
            repo,
            hub,
            id,
            status,
            order,
            worktree,
            issue,
            pr,
            note,
            json,
        } => cmd::task_update(&cmd::UpdateArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            id,
            status: status.as_deref(),
            order: *order,
            worktree: worktree.as_deref(),
            issue: issue.as_deref(),
            pr: pr.as_deref(),
            note: note.as_deref(),
            json: *json,
        }),
    }
}

/// The `adj gate` verbs, split out for the same reason `run_task` is.
fn run_gate(action: &GateAction) -> Result<(), String> {
    match action {
        GateAction::Open {
            repo,
            hub,
            file,
            json,
        } => cmd::gate_open(repo.as_deref(), hub.as_deref(), file.as_deref(), *json),
        GateAction::List { repo, hub, json } => {
            cmd::gate_list(repo.as_deref(), hub.as_deref(), *json)
        }
        GateAction::Show { repo, hub, id } => cmd::gate_show(repo.as_deref(), hub.as_deref(), id),
        GateAction::Answer {
            repo,
            hub,
            id,
            decision,
            choice,
            comment,
            json,
        } => cmd::gate_answer(&cmd::AnswerArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            id,
            decision,
            choice: choice.as_deref(),
            comment: comment.as_deref(),
            json: *json,
        }),
        GateAction::Close {
            repo,
            hub,
            id,
            comment,
            json,
        } => cmd::gate_close(&cmd::CloseArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            id,
            comment: comment.as_deref(),
            json: *json,
        }),
    }
}
