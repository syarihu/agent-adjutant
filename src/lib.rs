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
mod gate;
mod infra;
mod jules;
mod kernel;
mod mail;
mod mcp;
mod messaging;
mod registry;
mod session;
mod task;
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
                std::env::remove_var(crate::infra::env::STARTUP_DASHBOARD_ENV);
                // `cargo test` run from a git hook inherits `GIT_DIR`, and a test that sets
                // up a repository with git directly would then set up the developer's own.
                // Cleared here too so that one a test set and failed to take back ends with
                // the next sandbox.
                for name in crate::infra::git::REPOSITORY_LOCATION_ENV {
                    std::env::remove_var(name);
                }
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

    /// A repository at `dir` on `branch`, set up through `crate::infra::git::git` so that a variable a
    /// test has already exported cannot send the setup somewhere else.
    pub fn init_repo(dir: &std::path::Path, branch: &str) {
        let out = crate::infra::git::git(&["init", "-q", "-b", branch], Some(dir)).unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A variable set for as long as this lives, and removed after.
    ///
    /// Borrows the `Sandbox` for as long as it lives, so that it is only ever set while the
    /// environment lock is held: the variable is process-global, and a test running beside
    /// this one would see it.
    pub struct EnvVar<'a>(&'static str, std::marker::PhantomData<&'a Sandbox>);

    impl<'a> EnvVar<'a> {
        pub fn set(
            _sandbox: &'a Sandbox,
            name: &'static str,
            value: impl AsRef<std::ffi::OsStr>,
        ) -> Self {
            unsafe { std::env::set_var(name, value) };
            EnvVar(name, std::marker::PhantomData)
        }
    }

    impl Drop for EnvVar<'_> {
        fn drop(&mut self) {
            unsafe { std::env::remove_var(self.0) };
        }
    }
}

mod cli_args;

use clap::Parser;
use cli_args::{Cli, Commands, GateAction, JulesAction, ServerAction, TaskAction, TmuxAction};

fn resolve_wake_flag(wake: bool, no_wake: bool) -> Option<bool> {
    if wake {
        Some(true)
    } else if no_wake {
        Some(false)
    } else {
        None
    }
}

/// Parse this process's arguments and run the subcommand. Exits; never returns.
pub fn run() -> ! {
    kernel::config::anchor_config_env();
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
            wake,
            no_wake,
        } => cmd::send(&cmd::SendArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            from: from.as_deref(),
            kind,
            subject: subject.as_deref(),
            body: body.as_deref(),
            quiet: *quiet,
            wake: resolve_wake_flag(*wake, *no_wake),
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
            task,
            prompt,
            resume,
            dry_run,
        } => cmd::work(&cmd::WorkArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            worktree,
            title,
            task: task.as_deref(),
            prompt: prompt.as_deref(),
            resume: *resume,
            dry_run: *dry_run,
        }),
        // Exit 1 when the hub is not running, so a shell can branch on it without parsing
        // anything this prints.
        Commands::Worker {
            repo,
            hub,
            worktree,
            title,
            task,
            prompt,
            resume,
            dry_run,
        } => cmd::worker(&cmd::WorkerArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            worktree: worktree.as_deref(),
            title: title.as_deref(),
            task: task.as_deref(),
            prompt: prompt.as_deref(),
            resume: *resume,
            dry_run: *dry_run,
        })
        .map(|_| 0),
        Commands::Tell {
            repo,
            hub,
            worktree,
            subject,
            body,
            from,
            quiet,
            wake,
            no_wake,
        } => cmd::tell(&cmd::TellArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            worktree,
            subject,
            body: body.as_deref(),
            from: from.as_deref(),
            quiet: *quiet,
            wake: resolve_wake_flag(*wake, *no_wake),
        })
        .map(|_| 0),
        Commands::Serve {
            repo,
            hub,
            port,
            no_open,
        } => cmd::serve(repo.as_deref(), hub.as_deref(), *port, !*no_open).map(|_| 0),
        Commands::Server { action } => match action {
            ServerAction::Start {
                port,
                foreground,
                no_open,
            } => cmd::server_start(*port, *foreground, !*no_open),
            ServerAction::Stop => cmd::server_stop(),
            ServerAction::Restart { port, open } => cmd::server_restart(*port, *open),
            ServerAction::Status { json } => cmd::server_status(*json),
        },
        Commands::Gate { action } => run_gate(action).map(|_| 0),
        Commands::Task { action } => run_task(action).map(|_| 0),
        Commands::Jules { action } => run_jules(action).map(|_| 0),
        Commands::Skill {
            name,
            arguments,
            agent,
        } => cmd::skill(name, arguments, agent.as_deref()).map(|_| 0),
        Commands::Outbox { worktree, clear } => cmd::outbox(worktree.as_deref(), *clear).map(|_| 0),
        Commands::Focus {
            repo,
            hub: _,
            worktree: Some(worktree),
            quiet,
            dry_run,
        } => cmd::focus_worker_cmd(repo.as_deref(), worktree, *quiet, *dry_run)
            .map(|found| i32::from(!found)),
        Commands::Focus {
            repo,
            hub,
            worktree: None,
            quiet,
            dry_run,
        } => cmd::focus(repo.as_deref(), hub.as_deref(), *quiet, *dry_run)
            .map(|found| i32::from(!found)),
        Commands::Phase { set, worktree } => {
            cmd::phase(worktree.as_deref(), set.as_deref()).map(|_| 0)
        }
        Commands::ReviewEngine { repo, json } => {
            cmd::review_engine(repo.as_deref(), *json).map(|_| 0)
        }
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
        Commands::HubClose { repo, hub } => {
            cmd::hub_close(repo.as_deref(), hub.as_deref()).map(|_| 0)
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
            unique,
        } => cmd::worktree_path(&cmd::WorktreeArgs {
            repo: repo.as_deref(),
            branch: branch.as_deref(),
            name: name.as_deref(),
            user: user.as_deref(),
            pattern: pattern.as_deref(),
            unique: *unique,
        })
        .map(|_| 0),
        Commands::Mcp => mcp::run_server().map(|_| 0).map_err(|e| e.to_string()),
        Commands::InstallMcp { target } => mcp::install(target).map(|_| 0),
        Commands::UninstallMcp { target } => mcp::uninstall(target).map(|_| 0),
        Commands::Tmux { action } => run_tmux(action).map(|_| 0),
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

/// The `adj task` verbs. Split out of `run` because the verbs are a subcommand of a
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
            stop_at,
            executor,
            issue_url,
            base,
            parent,
            worktree_name,
            ask_first,
            queue,
            waiting_in,
            json,
        } => cmd::task_add(&cmd::AddArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            title: title.as_deref(),
            body: body.as_deref(),
            kind,
            done_when,
            stop_at,
            executor,
            issue_url: issue_url.as_deref(),
            base: base.as_deref(),
            parent: parent.as_deref(),
            worktree_name: worktree_name.as_deref(),
            ask_first: *ask_first,
            queue: *queue,
            waiting_in: waiting_in.as_deref(),
            json: *json,
        }),
        TaskAction::List {
            repo,
            hub,
            status,
            worktree,
            json,
        } => cmd::task_list(
            repo.as_deref(),
            hub.as_deref(),
            status.as_deref(),
            worktree.as_deref(),
            *json,
        ),
        TaskAction::Show { repo, hub, id } => cmd::task_show(repo.as_deref(), hub.as_deref(), id),
        TaskAction::Next { repo, hub, json } => {
            cmd::task_next(repo.as_deref(), hub.as_deref(), *json)
        }
        TaskAction::Refresh { repo, hub, json } => {
            cmd::task_refresh_cmd(repo.as_deref(), hub.as_deref(), *json)
        }
        TaskAction::FetchIssue {
            repo,
            hub,
            id,
            json,
        } => cmd::task_fetch_issue_cmd(repo.as_deref(), hub.as_deref(), id, *json),
        TaskAction::Update {
            repo,
            hub,
            id,
            status,
            order,
            worktree,
            issue,
            pr,
            base,
            jules_session,
            executor,
            note,
            instruction,
            auto_start,
            no_hand_over,
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
            base: base.as_deref(),
            jules_session: jules_session.as_deref(),
            executor: executor.as_deref(),
            note: note.as_deref(),
            instruction: instruction.as_deref(),
            auto_start: *auto_start,
            no_hand_over: *no_hand_over,
            json: *json,
        }),
        TaskAction::Brief {
            repo,
            hub,
            id,
            worktree,
            base,
            key,
            tracker,
            parent,
            instruction,
            out,
            json,
        } => cmd::task_brief(&cmd::BriefArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            id: id.as_deref(),
            worktree,
            base,
            key: key.as_deref(),
            tracker: tracker.as_deref(),
            parent: parent.as_deref(),
            instruction: instruction.as_deref(),
            out: out.as_deref(),
            json: *json,
        }),
    }
}

/// The `adj jules` verbs, split out for the same reason `run_task` is.
fn run_jules(action: &JulesAction) -> Result<(), String> {
    match action {
        JulesAction::Start {
            repo,
            hub,
            id,
            prompt_file,
            base,
            json,
        } => cmd::jules_start(&cmd::JulesStartArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            id,
            prompt: prompt_file,
            base: base.as_deref(),
            json: *json,
        }),
        JulesAction::Show {
            repo,
            hub,
            id,
            session,
            json,
        } => cmd::jules_show(&cmd::JulesShowArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            id: id.as_deref(),
            session: session.as_deref(),
            json: *json,
        }),
        JulesAction::Findings {
            repo,
            hub,
            id,
            json,
        } => cmd::jules_findings_cmd(repo.as_deref(), hub.as_deref(), id, *json),
        JulesAction::Relay {
            repo,
            hub,
            id,
            comments,
            plan_file,
            note,
        } => cmd::jules_relay_cmd(
            repo.as_deref(),
            hub.as_deref(),
            id,
            comments,
            plan_file.as_deref(),
            note.as_deref(),
        ),
    }
}

/// The `adj gate` verbs, split out for the same reason `run_task` is.
fn run_gate(action: &GateAction) -> Result<(), String> {
    match action {
        GateAction::Open {
            repo,
            hub,
            file,
            body_file,
            json,
        } => cmd::gate_open(
            repo.as_deref(),
            hub.as_deref(),
            file.as_deref(),
            body_file.as_deref(),
            *json,
        ),
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
            terminal,
            json,
        } => cmd::gate_close(&cmd::CloseArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            id,
            comment: comment.as_deref(),
            terminal: *terminal,
            json: *json,
        }),
    }
}

/// The `adj tmux` verbs.
fn run_tmux(action: &TmuxAction) -> Result<(), String> {
    match action {
        TmuxAction::Pane {
            repo,
            socket,
            pid,
            tty,
            json,
        } => cmd::tmux::pane(
            repo.as_deref(),
            socket.as_deref(),
            *pid,
            tty.as_deref(),
            *json,
        ),
        TmuxAction::Wake {
            repo,
            socket,
            pid,
            line,
            agent,
            dry_run,
        } => cmd::tmux::wake(
            repo.as_deref(),
            socket.as_deref(),
            *pid,
            line.as_deref(),
            agent.as_deref(),
            *dry_run,
        ),
        TmuxAction::Focus {
            repo,
            socket,
            pid,
            dry_run,
        } => cmd::tmux::focus(repo.as_deref(), socket.as_deref(), *pid, *dry_run),
        TmuxAction::Close {
            repo,
            socket,
            pid,
            dry_run,
        } => cmd::tmux::close(repo.as_deref(), socket.as_deref(), *pid, *dry_run),
        TmuxAction::Spawn {
            repo,
            socket,
            session,
            cwd,
            title,
            command,
            dry_run,
        } => cmd::tmux::spawn(
            repo.as_deref(),
            socket.as_deref(),
            session.as_deref(),
            cwd.as_deref(),
            title,
            &strip_separator(command),
            *dry_run,
        ),
    }
}
