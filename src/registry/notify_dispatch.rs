//! Telling the person something happened, through whoever is open to hear it.
//!
//! With `notification` unset (`Hook::BuiltIn`) the notice goes to the desktop app if it polled
//! within `PAGE_FRESH_SECS`, else to a board page that did (both ring it themselves, so it is
//! queued for them), else to `terminal-notifier` if it is installed, else nowhere. A configured
//! command, or `false`, is what it always was: nothing here looks at who is open.

use super::notify_clients::{Client, Route, notify_clients};
use super::*;
use crate::infra::notify;
use crate::infra::terminal::Hook;

/// What is to be done with one notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// `notification` is `false`.
    Off,
    /// Queue it for this client, which rings it with its next poll.
    Queue(Client),
    /// Run this command line.
    Run(String),
    /// Unset, and nobody is open and there is no `terminal-notifier`.
    NoNotifier,
}

/// What was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notified {
    Off,
    Queued(Client),
    Ran,
    NoNotifier,
}

/// What `notify` would do, without doing it: for a dry run. Reads who is open; writes nothing.
pub fn plan_notification(state: &Path, hook: &Hook, nwo: &str, title: &str, message: &str) -> Plan {
    plan_notification_with(
        state,
        hook,
        nwo,
        title,
        message,
        now_secs(),
        &notify::default_command,
    )
}

/// `plan_notification` with the clock and the built-in command given, so that a test can name
/// them rather than run `terminal-notifier` or wait on the clock.
pub fn plan_notification_with(
    state: &Path,
    hook: &Hook,
    nwo: &str,
    title: &str,
    message: &str,
    now: i64,
    built_in: &dyn Fn(&str, &str) -> Option<String>,
) -> Plan {
    if hook.is_off() {
        return Plan::Off;
    }
    if hook.template().is_some() {
        return notify::command(hook, nwo, title, message).map_or(Plan::Off, Plan::Run);
    }
    match notify_clients(state).route(now) {
        Route::App => Plan::Queue(Client::App),
        Route::Page => Plan::Queue(Client::Page),
        Route::None => built_in(title, message).map_or(Plan::NoNotifier, Plan::Run),
    }
}

/// Tell the person. A queued notice that cannot be written falls back to the command the
/// built-in has, so that it is not lost for want of a directory.  A command is run before this
/// returns, as the one it replaces was.
pub fn notify_person(
    state: &Path,
    hook: &Hook,
    nwo: &str,
    title: &str,
    message: &str,
) -> Result<Notified, String> {
    notify_person_with(
        state,
        hook,
        nwo,
        title,
        message,
        now_secs(),
        &notify::default_command,
        &|command| crate::infra::shell::run_shell(command).map(|_| ()),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn notify_person_with(
    state: &Path,
    hook: &Hook,
    nwo: &str,
    title: &str,
    message: &str,
    now: i64,
    built_in: &dyn Fn(&str, &str) -> Option<String>,
    run: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Notified, String> {
    match plan_notification_with(state, hook, nwo, title, message, now, built_in) {
        Plan::Off => Ok(Notified::Off),
        Plan::NoNotifier => Ok(Notified::NoNotifier),
        Plan::Run(command) => run(&command).map(|()| Notified::Ran),
        Plan::Queue(client) => match enqueue_notice(state, nwo, title, message, now) {
            Ok(_) => Ok(Notified::Queued(client)),
            Err(_) => match built_in(title, message) {
                Some(command) => run(&command).map(|()| Notified::Ran),
                None => Ok(Notified::NoNotifier),
            },
        },
    }
}

#[cfg(test)]
mod tests;
