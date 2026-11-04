use std::time::Duration;

/// Where the inbox, the records and the boards live. A constant for the same reason
/// `config::CONFIG_ENV` is one: it has to be forwarded by name into the tabs this opens.
pub const STATE_DIR_ENV: &str = "ADJUTANT_STATE_DIR";

/// Raises (never lowers) the short deadlines given to the outside programs a server start's
/// machine check runs and the one PR read a move to `done` makes. It exists for the
/// integration tests, whose stub programs can take seconds to start on a loaded machine.
/// A whole number of seconds; anything else (unset, empty, not a number, 0) leaves the
/// deadlines as they are.
pub const DEADLINE_FLOOR_ENV: &str = "ADJUTANT_DEADLINE_FLOOR_SECS";

/// What `deadline` becomes when raised by `floor`: `max(deadline, floor seconds)` when `floor`
/// parses as a `u64` greater than 0 (trimming whitespace), else `deadline` unchanged.
pub fn deadline_with_floor(deadline: Duration, floor: Option<&str>) -> Duration {
    let floor_secs = floor
        .map(str::trim)
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&s| s > 0);
    match floor_secs {
        Some(secs) => deadline.max(Duration::from_secs(secs)),
        None => deadline,
    }
}

/// `deadline` raised by `DEADLINE_FLOOR_ENV` when that variable is set.
pub fn deadline(deadline: Duration) -> Duration {
    deadline_with_floor(deadline, std::env::var(DEADLINE_FLOOR_ENV).ok().as_deref())
}

/// Overrides `startupDashboard` for one hub, set by `adj hub --no-dashboard` / `--dashboard`.
///
/// A flag on a command that `exec`s an agent has no other way to reach the prompt: the hub
/// reads its settings through `adjutant_config`, which is served by an MCP server that is
/// the agent's own child, so the environment is the one channel that survives both hops.
/// The same trick `ADJUTANT_HUB` uses, for the same reason.
///
/// `"1"` and `"0"` and nothing else. Anything else falls through to the configured value
/// rather than picking a side, because a variable somebody exported with a typo in it should
/// not quietly reverse a setting they wrote down on purpose.
pub const STARTUP_DASHBOARD_ENV: &str = "ADJUTANT_STARTUP_DASHBOARD";

// ── where the config lives ───────────────────────────────────────────

/// The file this binary reads its configuration out of. Named here rather than spelled in
/// each place that forwards it: a tab that is handed the wrong one reads a different world.
pub const CONFIG_ENV: &str = "ADJUTANT_CONFIG";
pub const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
pub const TMUX_SOCKET_ENV: &str = "ADJUTANT_TMUX_SOCKET";
pub const TMUX_SESSION_ENV: &str = "ADJUTANT_TMUX_SESSION";

// ── which hub is being addressed ─────────────────────────────────────

/// What `adj hub --hub` was told, handed down to the agent it starts.
///
/// The hub's own procedure calls `adjutant_pending` and friends with no arguments at all —
/// it is talking about itself, and a rule that says "pass your own name every time" is a
/// rule that gets forgotten once and then silently reads somebody else's inbox. An
/// environment variable is the one channel that reaches every one of those calls without
/// any of them mentioning it: `adj hub` puts it on the line it `exec`s, the agent inherits
/// it, and the MCP server the agent starts is that agent's child.
pub const HUB_ENV: &str = "ADJUTANT_HUB";

/// Carried on the hub's command line and inherited by the MCP server the agent starts:
/// `{slug}/{session id}`. It is what tells that server it belongs to a hub — the same server
/// runs under every session on the machine that has it registered — and which one.
pub const HUB_SESSION_ENV: &str = "ADJUTANT_HUB_SESSION";

/// The session a reader of a pull request runs as, set on the line `adj review run` execs.
/// `adj review submit` reads it so that a hand-in from a reader that was superseded (the read
/// was asked for again) is refused. A person handing a file in by hand sets none.
pub const REVIEW_SESSION_ENV: &str = "ADJUTANT_REVIEW_SESSION";

/// Carried on the hub's command line like `HUB_SESSION_ENV`, and holding the hub's slug:
/// it is what tells the MCP server under the agent to serve that hub's board. A variable of
/// its own rather than the session one, because that one is left off for a runner that
/// records no session, and such a hub still wants its board. Left off when `hubServe` is
/// `false`.
pub const HUB_SERVE_ENV: &str = "ADJUTANT_HUB_SERVE";

// ── what Claude Code gives a hook ────────────────────────────────────

/// The pid of the `claude` process, set in a hook's environment (documented from Claude Code
/// 2.1.214). Read by the hook receiver to join a session's row to the process behind it.
pub const CLAUDE_PID_ENV: &str = "CLAUDE_PID";

/// Where the session's Claude Code account keeps its files. Unset on the default account, whose
/// directory is `~/.claude`.
pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

// ── what Codex gives a hook ──────────────────────────────────────────

/// Where Codex keeps its files (its `hooks.json` among them). Unset on the default install,
/// whose directory is `~/.codex`.
pub const CODEX_HOME_ENV: &str = "CODEX_HOME";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deadline_floor_only_ever_raises_a_deadline() {
        let default = Duration::from_secs(5);

        // unset
        assert_eq!(deadline_with_floor(default, None), default);

        // empty
        assert_eq!(deadline_with_floor(default, Some("")), default);
        assert_eq!(deadline_with_floor(default, Some("   ")), default);

        // garbage
        assert_eq!(deadline_with_floor(default, Some("abc")), default);
        assert_eq!(deadline_with_floor(default, Some("12abc")), default);

        // 0
        assert_eq!(deadline_with_floor(default, Some("0")), default);
        assert_eq!(deadline_with_floor(default, Some("  0  ")), default);

        // a smaller value (deadline kept)
        assert_eq!(
            deadline_with_floor(Duration::from_secs(10), Some("5")),
            Duration::from_secs(10)
        );

        // a larger value (floor used)
        assert_eq!(
            deadline_with_floor(default, Some("60")),
            Duration::from_secs(60)
        );

        // whitespace
        assert_eq!(
            deadline_with_floor(default, Some(" \t 60 \n")),
            Duration::from_secs(60)
        );
    }
}
