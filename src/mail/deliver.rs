use super::*;
use crate::infra::notify;
use crate::infra::terminal::{self, Wake};
use crate::kernel::config::Settings;

/// What a message came to, given whether anyone was there to read it, whether it called for a
/// wake, and what the wake came to when one was tried: whether the session was woken, and,
/// when the screen is what stopped it, the reason. Any other failure is left unsaid, as it
/// always was.
///
/// The wake is run only when someone is there and it is called for.
pub(crate) fn reached_after(
    present: bool,
    wake_needed: bool,
    wake: impl FnOnce() -> Option<Result<terminal::Performed, String>>,
) -> Reached {
    if !present {
        return Reached::NotRunning;
    }
    if !wake_needed {
        return Reached::Running {
            wake: NotWoken::NotNeeded,
        };
    }
    match wake() {
        Some(Ok(done)) if done.ran => Reached::Woken,
        Some(Ok(done)) if done.screen => Reached::Running {
            wake: NotWoken::Held {
                why: Some(done.description),
            },
        },
        _ => Reached::Running {
            wake: NotWoken::Held { why: None },
        },
    }
}

/// What a wake in one direction is made with: the hook, the built-in line, the runner whose
/// screen is read.
fn wake_settings(settings: &Settings, to_hub: bool) -> (&Wake, &'static str, Option<&str>) {
    if to_hub {
        (
            &settings.hub_wake,
            terminal::HUB_WAKE_LINE,
            settings.hub_runner.as_deref(),
        )
    } else {
        (
            &settings.worker_wake,
            terminal::WORKER_WAKE_LINE,
            settings.agent_runner.as_deref(),
        )
    }
}

/// Whether the built-in wake will read the receiver's screen before typing.
pub fn wake_looks_at_screen(settings: &Settings, to_hub: bool) -> bool {
    let (wake, _, runner) = wake_settings(settings, to_hub);
    !wake.hook.is_off()
        && wake.hook.template().is_none()
        && settings.terminal.is_tmux()
        && look_before_typing(wake_agent(runner)).is_some()
}

/// Leave a message for the hub, poke its tab if needed, and tell the person.
///
/// Shared by `adj send` and by the dashboard's hand-over, which is the whole reason it is a
/// function: the three steps are one rule, and a second copy of it is a second set of
/// conditions about when to wake and when to notify — drifting from the day it is written.
pub fn deliver_to_hub(ctx: &Context, message: &Message) -> Result<DeliveryOutcome, String> {
    deliver_to_hub_with_wake(ctx, message, true, None)
}

/// `deliver_to_hub`, choosing whether to tell the person. Not when they are the sender: a
/// decision made on the board a moment ago does not need a banner to say it was made.
pub fn deliver_to_hub_announcing(
    ctx: &Context,
    message: &Message,
    announce: bool,
) -> Result<DeliveryOutcome, String> {
    deliver_to_hub_with_wake(ctx, message, announce, None)
}

/// `deliver_to_hub`, choosing whether to announce and optionally overriding the wake decision.
pub fn deliver_to_hub_with_wake(
    ctx: &Context,
    message: &Message,
    announce: bool,
    wake: Option<bool>,
) -> Result<DeliveryOutcome, String> {
    Ok(post_to_hub_with_wake(ctx, message, wake)?.follow_up(ctx, announce))
}

/// A message written into the hub's inbox, its two follow-ups not yet run.
pub struct Posted {
    subject: String,
    delivery: Delivery,
    wake_needed: bool,
}

/// The first half of `deliver_to_hub`: the message is in the inbox once this returns.
///
/// Split off for a caller holding a lock: writing a file is quick, while waking the hub and
/// notifying run commands of the person's choosing, which can hang. Such a caller posts under
/// the lock and follows up after letting it go.
pub fn post_to_hub(ctx: &Context, message: &Message) -> Result<Posted, String> {
    post_to_hub_with_wake(ctx, message, None)
}

pub fn post_to_hub_with_wake(
    ctx: &Context,
    message: &Message,
    wake: Option<bool>,
) -> Result<Posted, String> {
    let subject = header_value(&render_message(message), "subject").unwrap_or_default();
    let delivery = send(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name, message)?;
    let wake_needed = wake.unwrap_or_else(|| {
        should_wake_hub(&message.from, &ctx.repo.hub_name, &message.kind, &subject)
    });
    Ok(Posted {
        subject,
        delivery,
        wake_needed,
    })
}

impl Posted {
    /// The second half: poke the hub if it is there and waking is needed, and tell the person when `announce`.
    pub fn follow_up(self, ctx: &Context, announce: bool) -> DeliveryOutcome {
        let Posted {
            subject,
            delivery,
            wake_needed,
        } = self;

        let (wake, default_line, runner) = wake_settings(&ctx.settings, true);
        let reached = reached_after(delivery.present, wake_needed, || {
            hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name)
                .pid
                .map(|pid| {
                    terminal::wake(
                        &ctx.settings.terminal,
                        wake,
                        pid,
                        &subject,
                        default_line,
                        look_before_typing(wake_agent(runner)),
                        false,
                    )
                })
        });

        if announce
            && wake_needed
            && let Some(command) = notify::repo_command(
                &ctx.settings.notification,
                &ctx.repo.nwo,
                &ctx.repo.repo,
                &subject,
            )
        {
            let _ = crate::infra::shell::run_shell(&command);
        }
        DeliveryOutcome {
            path: delivery.path,
            reached,
        }
    }
}

/// Append to a worktree's outbox, poke the worker sitting in it if waking is needed,
/// and tell the person when poking was not possible.
///
/// Shared by `adj tell` and by a gate's answer, which is the point: both are the hub-to-
/// worker direction, and the rule about when to wake and when to notify is one rule. A
/// worker that was woken reads the message itself, so the notification is what happens
/// *instead* — unlike the other direction, where the hub is unattended and the person is
/// told either way.
pub fn deliver_to_worker(
    ctx: &Context,
    worktree: &std::path::Path,
    from: &str,
    subject: &str,
    body: &str,
    wake: Option<bool>,
) -> Result<DeliveryOutcome, String> {
    let path = tell(worktree, from, subject, body)?;
    let status = worker_status(worktree);
    let wake_needed = wake.unwrap_or_else(|| should_wake_worker(subject));
    let (wake, default_line, runner) = wake_settings(&ctx.settings, false);
    let reached = reached_after(status.present, wake_needed, || {
        status.pid.map(|pid| {
            terminal::wake(
                &ctx.settings.terminal,
                wake,
                pid,
                subject,
                default_line,
                look_before_typing(wake_agent(runner)),
                false,
            )
        })
    });
    if wake_needed
        && !matches!(reached, Reached::Woken)
        && let Some(command) = notify::repo_command(
            &ctx.settings.notification,
            &ctx.repo.nwo,
            &ctx.repo.repo,
            subject,
        )
    {
        let _ = crate::infra::shell::run_shell(&command);
    }
    Ok(DeliveryOutcome { path, reached })
}
