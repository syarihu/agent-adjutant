//! The dispatch: each clap variant taken apart and handed to the command that runs it.

use clap::Parser;

use super::args::{
    Cli, Commands, GateAction, JulesAction, ServerAction, TaskAction, TmuxAction, strip_separator,
};
use super::outbox;
use crate::transport::cli;

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
    crate::kernel::config::anchor_config_env();
    let cli = Cli::parse();
    let result: Result<i32, String> = match &cli.command {
        Commands::HubName { repo, hub, json } => {
            super::hub_name(repo.as_deref(), hub.as_deref(), *json).map(|_| 0)
        }
        Commands::Config { repo, hub } => {
            super::show_config(repo.as_deref(), hub.as_deref()).map(|_| 0)
        }
        Commands::Pending {
            repo,
            hub,
            path,
            limit,
            json,
            read,
            ack,
        } => super::pending(&super::PendingArgs {
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
        } => super::send(&super::SendArgs {
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
        } => super::spawn(
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
        } => super::work(&super::WorkArgs {
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
        } => super::worker(&super::WorkerArgs {
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
        } => super::tell(&super::TellArgs {
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
        } => super::serve(repo.as_deref(), hub.as_deref(), *port, !*no_open).map(|_| 0),
        Commands::Server { action } => match action {
            ServerAction::Start(args) => super::server_start(args),
            ServerAction::Stop => super::server_stop(),
            ServerAction::Restart(args) => super::server_restart(args),
            ServerAction::Status(args) => super::server_status(args),
        },
        Commands::Gate { action } => run_gate(action).map(|_| 0),
        Commands::Task { action } => run_task(action).map(|_| 0),
        Commands::Jules { action } => run_jules(action).map(|_| 0),
        Commands::Skill {
            name,
            arguments,
            agent,
        } => super::skill(name, arguments, agent.as_deref()).map(|_| 0),
        Commands::Outbox { worktree, clear } => outbox(worktree.as_deref(), *clear).map(|_| 0),
        Commands::Focus {
            repo,
            hub: _,
            worktree: Some(worktree),
            quiet,
            dry_run,
        } => super::focus_worker_cmd(repo.as_deref(), worktree, *quiet, *dry_run)
            .map(|found| i32::from(!found)),
        Commands::Focus {
            repo,
            hub,
            worktree: None,
            quiet,
            dry_run,
        } => super::focus(repo.as_deref(), hub.as_deref(), *quiet, *dry_run)
            .map(|found| i32::from(!found)),
        Commands::Phase { set, worktree } => {
            super::phase(worktree.as_deref(), set.as_deref()).map(|_| 0)
        }
        Commands::ReviewEngine { repo, json } => {
            super::review_engine(repo.as_deref(), *json).map(|_| 0)
        }
        Commands::Close {
            repo,
            worktree,
            quiet,
            dry_run,
        } => {
            cli::close(repo.as_deref(), worktree, *quiet, *dry_run).map(|closed| i32::from(!closed))
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
        } => super::hub(
            repo.as_deref(),
            hub.as_deref(),
            &strip_separator(extra),
            *tab,
            match (*resume, *new) {
                (true, _) => crate::lifecycle::hub::HubStart::Resume,
                (_, true) => crate::lifecycle::hub::HubStart::New,
                _ => crate::lifecycle::hub::HubStart::Auto,
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
            super::hub_stop(repo.as_deref(), hub.as_deref()).map(|_| 0)
        }
        Commands::HubClose { repo, hub } => {
            super::hub_close(repo.as_deref(), hub.as_deref()).map(|_| 0)
        }
        Commands::Ide {
            repo,
            worktree,
            dry_run,
        } => super::open_ide(repo.as_deref(), worktree, *dry_run).map(|_| 0),
        Commands::Title {
            repo,
            title,
            dry_run,
        } => super::set_title(repo.as_deref(), title, *dry_run).map(|_| 0),
        Commands::Notify {
            repo,
            title,
            message,
            dry_run,
        } => super::notify_user(repo.as_deref(), title, message, *dry_run).map(|_| 0),
        Commands::WorktreePath {
            repo,
            branch,
            name,
            user,
            pattern,
            unique,
        } => super::worktree_path(&super::WorktreeArgs {
            repo: repo.as_deref(),
            branch: branch.as_deref(),
            name: name.as_deref(),
            user: user.as_deref(),
            pattern: pattern.as_deref(),
            unique: *unique,
        })
        .map(|_| 0),
        Commands::Mcp => crate::transport::mcp::run_server()
            .map(|_| 0)
            .map_err(|e| e.to_string()),
        Commands::InstallMcp { target } => crate::transport::mcp::install(target).map(|_| 0),
        Commands::UninstallMcp { target } => crate::transport::mcp::uninstall(target).map(|_| 0),
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

/// The `adj task` verbs. Split out of `run` because the verbs are a subcommand of a
/// subcommand, and inlining that match would bury the rest of the dispatch.
fn run_task(action: &TaskAction) -> Result<(), String> {
    match action {
        TaskAction::Add(args) => super::task_add(args),
        TaskAction::List(args) => super::task_list(args),
        TaskAction::Show(args) => super::task_show(args),
        TaskAction::Next(args) => super::task_next(args),
        TaskAction::Refresh(args) => super::task_refresh_cmd(args),
        TaskAction::FetchIssue(args) => super::task_fetch_issue_cmd(args),
        TaskAction::Update(args) => super::task_update(args),
        TaskAction::Brief(args) => super::task_brief(args),
    }
}

/// The `adj jules` verbs, split out for the same reason `run_task` is.
fn run_jules(action: &JulesAction) -> Result<(), String> {
    match action {
        JulesAction::Start(args) => super::jules_start(args),
        JulesAction::Show(args) => super::jules_show(args),
        JulesAction::Findings(args) => super::jules_findings_cmd(args),
        JulesAction::Relay(args) => super::jules_relay_cmd(args),
    }
}

/// The `adj gate` verbs, split out for the same reason `run_task` is.
fn run_gate(action: &GateAction) -> Result<(), String> {
    match action {
        GateAction::Open(args) => super::gate_open(args),
        GateAction::List(args) => super::gate_list(args),
        GateAction::Show(args) => super::gate_show(args),
        GateAction::Answer(args) => super::gate_answer(args),
        GateAction::Close(args) => super::gate_close(args),
    }
}

/// The `adj tmux` verbs.
fn run_tmux(action: &TmuxAction) -> Result<(), String> {
    match action {
        TmuxAction::Pane(args) => super::tmux::pane(args),
        TmuxAction::Wake(args) => super::tmux::wake(args),
        TmuxAction::Focus(args) => super::tmux::focus(args),
        TmuxAction::Close(args) => super::tmux::close(args),
        TmuxAction::Spawn(args) => super::tmux::spawn(args),
    }
}
