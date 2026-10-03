/// Where the inbox, the records and the boards live. A constant for the same reason
/// `config::CONFIG_ENV` is one: it has to be forwarded by name into the tabs this opens.
pub const STATE_DIR_ENV: &str = "ADJUTANT_STATE_DIR";

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

/// Carried on the hub's command line like `HUB_SESSION_ENV`, and holding the hub's slug:
/// it is what tells the MCP server under the agent to serve that hub's board. A variable of
/// its own rather than the session one, because that one is left off for a runner that
/// records no session, and such a hub still wants its board. Left off when `hubServe` is
/// `false`.
pub const HUB_SERVE_ENV: &str = "ADJUTANT_HUB_SERVE";
