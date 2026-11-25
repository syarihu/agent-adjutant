//! Ringing the configured `notification` for a gate that opens while no page that may notify is
//! open. With one open, the page rings the gate itself (`checkNewGates`), so nothing is claimed
//! and the gate is looked at again next round, in case the page lapses while it is still open.
//! With `notification` unset the target is the app, else a page, wherever it is open: a client of
//! another process cannot see this hub's gates, so a notice is queued for it (once, behind the
//! gate's marker), and nothing rings when its 確認待ち is off.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{RETRY_SECS, Ring, WaitWatch, built_in_ring, claim, route, safe_name};
use crate::board::{Server, settings_now};
use crate::gate::{self, Gate};
use crate::infra::clock::utc_stamp;
use crate::infra::notify;
use crate::infra::terminal::Hook;
use crate::registry::WaitRoute;

/// What the gate sweep has looked at.
#[derive(Default)]
pub(super) struct GateState {
    /// When the sweep first ran: a gate already open then is not rung, as the page does not ring
    /// for the ones it finds when it opens.
    since: Option<String>,
    /// The gates rung or given up on, by hub slug and id, for as long as they stay open.
    handled: HashSet<(String, String)>,
}

/// The listed gates that opened at or after `since` and were not handled, with the hub slug each
/// is on.
fn pending_gates(
    listed: &[(String, Vec<Gate>)],
    since: &str,
    handled: &HashSet<(String, String)>,
) -> Vec<(String, Gate)> {
    listed
        .iter()
        .flat_map(|(slug, gates)| gates.iter().map(move |gate| (slug, gate)))
        .filter(|(slug, gate)| {
            gate.opened_at.as_str() >= since
                && !handled.contains(&((*slug).clone(), gate.id.clone()))
        })
        .map(|(slug, gate)| (slug.clone(), gate.clone()))
        .collect()
}

/// What the configured notification says of a gate.
fn gate_message(gate: &Gate) -> String {
    let title = gate.title.trim();
    let kind = gate.kind.as_str();
    let who = Path::new(&gate.worktree)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty());
    let head = match who {
        Some(who) => format!("{who} opened a {kind} gate"),
        None => format!("New {kind} gate"),
    };
    if title.is_empty() {
        head
    } else {
        format!("{head}: {title}")
    }
}

/// What the page rings a gate under (`gate-` and the ref `<board slug>/<gate id>`, `checkNewGates`
/// in core.js): a notice queued for it carries the same, so a page that also reads the gate from
/// its own board's state shows one of the two.
fn gate_tag(slug: &str, id: &str) -> String {
    format!("gate-{slug}/{id}")
}

/// Where the claim on one gate is kept, as `marker_path` does for a wait: a gate id is only
/// unique within its hub.
fn gate_marker(root: &Path, slug: &str, id: &str) -> PathBuf {
    root.join("gate-notified")
        .join(format!("{}-{}", safe_name(slug), safe_name(id)))
}

impl WaitWatch {
    /// The commands that ring the gates opened since the sweep began, each once. Nothing is
    /// claimed while a page that may notify is open: it rings them itself. `boards` is asked
    /// only when there is a gate to ring.
    pub(super) fn gate_round(
        &self,
        root: &Path,
        slugs: &[String],
        boards: &impl Fn() -> (Vec<Arc<Server>>, bool),
        now: i64,
    ) -> Vec<String> {
        let listed: Vec<(String, Vec<Gate>)> = slugs
            .iter()
            .map(|slug| (slug.clone(), gate::list(root, slug, gate::Shelf::Open)))
            .collect();
        let pending = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let gates = &mut state.gates;
            let since = gates.since.get_or_insert_with(|| utc_stamp(now)).clone();
            let open: HashSet<(String, String)> = listed
                .iter()
                .flat_map(|(slug, list)| list.iter().map(move |g| (slug.clone(), g.id.clone())))
                .collect();
            gates.handled.retain(|key| open.contains(key));
            pending_gates(&listed, &since, &gates.handled)
        };
        // Rung by nobody here while a client is open to ring it, whatever the board's
        // `notification` is; which clients leave it to is a board's setting, asked below.
        let wait_at = crate::registry::wait_route(
            crate::registry::notify_clients(root),
            crate::registry::local_notify_clients(root),
            now,
        );
        if pending.is_empty() || (wait_at == WaitRoute::Leave && self.page_fresh(now)) {
            return Vec::new();
        }
        let (servers, complete) = boards();
        let given_up = utc_stamp(now - RETRY_SECS);
        let mut handled: Vec<(String, String)> = Vec::new();
        let mut rings: Vec<String> = Vec::new();
        for (slug, gate) in pending {
            let Some(server) = servers.iter().find(|s| s.ctx.repo.slug == slug) else {
                // A board that could not be opened is not a hub without a board, until the gate
                // is older than the retry window.
                if complete || gate.opened_at < given_up {
                    handled.push((slug, gate.id));
                }
                continue;
            };
            // Left to a client that is open: not claimed, so that it is rung here if they lapse.
            let settings = settings_now(server);
            if self.left_to_client(&settings.notification, wait_at, now) {
                continue;
            }
            let (ring, _) = route(false, false, false, || {
                claim(&gate_marker(root, &slug, &gate.id))
            });
            let ring = if matches!(settings.notification, Hook::BuiltIn) {
                built_in_ring(ring, wait_at)
            } else {
                ring
            };
            if ring == Ring::Queue {
                // The app or page that is open on another process cannot see this gate.
                let repo = &server.ctx.repo;
                let _ = crate::registry::enqueue_notice_tagged(
                    root,
                    &repo.nwo,
                    &notify::repo_title(&repo.repo),
                    &gate_message(&gate),
                    now,
                    Some(&gate_tag(&slug, &gate.id)),
                );
            }
            if ring == Ring::Now {
                let repo = &server.ctx.repo;
                if let Some(command) = notify::repo_command(
                    &settings.notification,
                    &repo.nwo,
                    &repo.repo,
                    &gate_message(&gate),
                ) {
                    rings.push(command);
                }
            }
            handled.push((slug, gate.id));
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.gates.handled.extend(handled);
        rings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::Kind;

    fn gate(id: &str, kind: Kind, worktree: &str, title: &str, opened_at: &str) -> Gate {
        let mut g: Gate = serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": kind.as_str(),
            "worktree": worktree,
            "title": title,
            "openedAt": opened_at,
        }))
        .unwrap();
        g.id = id.to_string();
        g
    }

    #[test]
    fn only_a_gate_opened_since_and_not_handled_is_pending() {
        let old = gate("old", Kind::Plan, "/w/a", "t", "20260101T000000Z");
        let new = gate("new", Kind::Plan, "/w/a", "t", "20260101T000010Z");
        let done = gate("done", Kind::Plan, "/w/a", "t", "20260101T000020Z");
        let listed = vec![("s".to_string(), vec![old, new.clone(), done])];
        let handled: HashSet<(String, String)> = [("s".to_string(), "done".to_string())].into();
        let pending = pending_gates(&listed, "20260101T000005Z", &handled);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.id, "new");
        // The same id on another hub is another gate.
        let handled: HashSet<(String, String)> = [("t".to_string(), "new".to_string())].into();
        assert_eq!(
            pending_gates(&listed, "20260101T000005Z", &handled).len(),
            2
        );
    }

    #[test]
    fn the_message_says_who_opened_which_gate() {
        let g = |worktree, title| gate("g", Kind::Plan, worktree, title, "20260101T000000Z");
        assert_eq!(
            gate_message(&g("/w/worker-a", "Add a flag")),
            "worker-a opened a plan gate: Add a flag"
        );
        assert_eq!(
            gate_message(&g("", "Add a flag")),
            "New plan gate: Add a flag"
        );
        assert_eq!(
            gate_message(&g("/w/worker-a", "  ")),
            "worker-a opened a plan gate"
        );
        let result = gate("g", Kind::Result, "/w/b", "Found it", "20260101T000000Z");
        assert_eq!(gate_message(&result), "b opened a result gate: Found it");
    }

    #[test]
    fn a_queued_gate_is_named_as_the_page_names_its_own() {
        assert_eq!(gate_tag("acme-widget", "g7"), "gate-acme-widget/g7");
    }

    #[test]
    fn a_gate_is_claimed_once_per_hub_and_id_and_stays_under_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let marker = gate_marker(dir.path(), "o-r", "x/../y");
        assert!(claim(&marker));
        assert!(!claim(&marker));
        assert!(claim(&gate_marker(dir.path(), "o-r2", "x/../y")));
        assert!(marker.starts_with(dir.path().join("gate-notified")));
    }
}
