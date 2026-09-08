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
mod ide;
mod mcp;
mod messaging;
mod notify;
mod prompts;
mod repo;
mod runner;
mod template;
mod terminal;

/// Scaffolding the tests share. Not a layer — nothing outside `#[cfg(test)]` may reach it,
/// which is why `check-layering.sh` lets any module name it.
#[cfg(test)]
pub(crate) mod testing {
    /// `ADJUTANT_CONFIG` and `ADJUTANT_STATE_DIR` are process-global and the harness runs
    /// tests in parallel threads, so a sandbox has to be exclusive or two tests read each
    /// other's config and each other's inboxes.
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
        /// Also print the slug, main checkout and where the name came from
        #[arg(long)]
        json: bool,
    },
    /// The resolved configuration for this repository, as JSON
    Config {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Messages waiting for this repository's hub
    Pending {
        #[arg(long)]
        repo: Option<String>,
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
        /// Who is sending: your session or worktree name
        #[arg(long)]
        from: Option<String>,
        /// report | question | answer | ack | needs-user
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
        /// What the worker is told on startup
        #[arg(
            long,
            default_value = ".claude/task-brief.md を読んで、その指示に従って作業を開始してください"
        )]
        prompt: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Start the worker agent in this tab (what `work` opens a tab to run)
    Worker {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(
            long,
            default_value = ".claude/task-brief.md を読んで、その指示に従って作業を開始してください"
        )]
        prompt: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Leave a message for the worker in a worktree (body from --body or stdin)
    Tell {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        worktree: String,
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
        /// Extra arguments appended to the agent command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },
    /// Clear this repository's hub record
    HubStop {
        #[arg(long)]
        repo: Option<String>,
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
        /// claude-code | json
        #[arg(long, default_value = "claude-code")]
        target: String,
    },
    /// Remove the MCP server registration
    UninstallMcp {
        #[arg(long, default_value = "claude-code")]
        target: String,
    },
}

/// Parse this process's arguments and run the subcommand. Exits; never returns.
pub fn run() -> ! {
    let cli = Cli::parse();
    let result: Result<i32, String> = match &cli.command {
        Commands::HubName { repo, json } => cmd::hub_name(repo.as_deref(), *json).map(|_| 0),
        Commands::Config { repo } => cmd::show_config(repo.as_deref()).map(|_| 0),
        Commands::Pending {
            repo,
            path,
            limit,
            json,
            read,
            ack,
        } => cmd::pending(&cmd::PendingArgs {
            repo: repo.as_deref(),
            path_only: *path,
            limit: *limit,
            as_json: *json,
            read: read.as_deref(),
            ack: ack.as_deref(),
        })
        .map(|_| 0),
        Commands::Send {
            repo,
            from,
            kind,
            subject,
            body,
            quiet,
        } => cmd::send(&cmd::SendArgs {
            repo: repo.as_deref(),
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
            worktree,
            title,
            prompt,
            dry_run,
        } => cmd::work(repo.as_deref(), worktree, title, prompt, *dry_run).map(|_| 0),
        // Exit 1 when the hub is not running, so a shell can branch on it without parsing
        // anything this prints.
        Commands::Worker {
            repo,
            worktree,
            title,
            prompt,
            dry_run,
        } => cmd::worker(repo.as_deref(), worktree, title, prompt, *dry_run).map(|_| 0),
        Commands::Tell {
            repo,
            worktree,
            subject,
            body,
            from,
            quiet,
        } => cmd::tell(
            repo.as_deref(),
            worktree,
            subject,
            body.as_deref(),
            from.as_deref(),
            *quiet,
        )
        .map(|_| 0),
        Commands::Skill { name, arguments } => cmd::skill(name, arguments).map(|_| 0),
        Commands::Outbox { worktree, clear } => cmd::outbox(worktree.as_deref(), *clear).map(|_| 0),
        Commands::Focus {
            repo,
            quiet,
            dry_run,
        } => cmd::focus(repo.as_deref(), *quiet, *dry_run).map(|found| i32::from(!found)),
        Commands::Hub {
            repo,
            dry_run,
            extra,
        } => cmd::hub(repo.as_deref(), &strip_separator(extra), *dry_run).map(|_| 0),
        Commands::HubStop { repo } => cmd::hub_stop(repo.as_deref()).map(|_| 0),
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
