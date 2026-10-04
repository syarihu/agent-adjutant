use super::*;

pub fn status_json(status: &HubStatus) -> Value {
    json!({
        "hubName": status.hub_name,
        "slug": status.slug,
        "present": status.present,
        "stale": status.stale,
        "pid": status.pid,
        "cwd": status.cwd,
        "startedAt": status.started_at,
        "inbox": inbox_dir(&status.slug).to_string_lossy(),
    })
}
