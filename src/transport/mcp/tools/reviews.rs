use serde_json::{Value, json};

use super::text_param;

/// The states in which a PR waits on the person: what `OTHERS_LIVE` is on the page.
const WAITING_ON_YOU: [&str; 4] = ["requested", "ai-reading", "ai-ready", "pushed"];

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_reviews",
        "description": "The code reviews the person is asked to do: what 「AI 下読み」 lists, the pull requests of other people that wait on the person's review. Not the review handling of the person's own task PRs: those are in adjutant_work. Per PR: `id` (owner/repo#N), `repo`, `title`, `url`, `author`, `state`, `rerequest` (asked again after a review), `requestedAt`, `events` (when each thing happened), and `ai`, where the 下読み stands (`stage` none, queued, running, ready, stale or failed, with `counts` when read). Read only; it marks nothing as seen. It needs the resident server running (`adj server start`). Show the list with adjutant_show kind=reviews, or one PR's read-through with kind=read.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "owner/name. Leave it out for every repository; with it, only that repository's requests are returned." },
            },
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let root = crate::registry::state_root(None);
    let doc = crate::board::ask_resident(&root, "/api/others")?;
    Ok(reviews_of(&doc, text_param(args, "repo")))
}

/// Where the AI 下読み of a record stands, in the words of the page's stages plus the two it keeps apart.
fn ai_of(rec: &Value) -> Value {
    let Some(ai) = rec.get("ai").filter(|a| a.is_object()) else {
        return json!({ "stage": "none" });
    };
    let stage = match (ai["status"].as_str(), ai["stale"].as_bool()) {
        (Some("queued"), _) => "queued",
        (Some("running"), _) => "running",
        (Some("ready"), Some(true)) => "stale",
        (Some("ready"), _) => "ready",
        (Some("failed"), _) => "failed",
        _ => "none",
    };
    let mut out = json!({ "stage": stage });
    for key in ["counts", "sha", "finishedAt", "mode", "error"] {
        if !ai[key].is_null() {
            out[key] = ai[key].clone();
        }
    }
    out
}

/// What the list of the resident (`GET /api/others`) holds for the person to review: the records that wait on them (not those
/// waiting on the author, nor done), whose own-task PRs are only there when someone also requested them of the person, as the list
/// has it. `repo` narrows by repository ignoring case.
fn reviews_of(doc: &Value, repo: Option<&str>) -> Value {
    let records = doc["records"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let state = |r: &Value| r["state"].as_str().unwrap_or("").to_string();
    let mut requests: Vec<Value> = records
        .iter()
        .filter(|r| WAITING_ON_YOU.contains(&state(r).as_str()))
        .filter(|r| {
            repo.is_none_or(|w| {
                r["repo"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(w))
            })
        })
        .map(|r| {
            let mut out = json!({ "ai": ai_of(r) });
            for key in [
                "id",
                "repo",
                "number",
                "title",
                "url",
                "author",
                "draft",
                "state",
                "requested",
                "rerequest",
                "requestedAt",
                "firstSeenAt",
                "readAt",
                "events",
                "prState",
            ] {
                out[key] = r[key].clone();
            }
            out
        })
        .collect();
    requests.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["repo"].as_str().unwrap_or("").to_ascii_lowercase(),
                v["requestedAt"].as_str().unwrap_or("~").to_string(),
                v["number"].as_u64(),
            )
        };
        key(a).cmp(&key(b))
    });
    let count = |want: &str| records.iter().filter(|r| state(r) == want).count();
    json!({
        "now": doc["now"],
        "lastSync": doc["lastSync"],
        "requests": requests,
        "waitingOnAuthor": count("waiting-on-author"),
        "done": count("done"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, repo: &str, state: &str, extra: Value) -> Value {
        let mut r = json!({"id": id, "repo": repo, "number": 1, "title": "T", "url": "u", "author": "alice", "draft": false,
            "state": state, "requested": true, "rerequest": false, "requestedAt": "20261001T000000Z", "prState": "open"});
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    }

    #[test]
    fn what_waits_on_the_person_is_listed_with_its_reading_and_the_rest_only_counted() {
        let doc = json!({"now": 5, "lastSync": {"at": "x"}, "records": [
            rec("b/y#2", "B/Y", "ai-ready", json!({"rerequest": true, "ai": {"status": "ready", "sha": "s", "counts": {"attention": 1}, "stale": false}})),
            rec("a/x#1", "A/X", "requested", json!({})),
            rec("a/x#3", "A/X", "pushed", json!({"ai": {"status": "ready", "stale": true}})),
            rec("a/x#4", "A/X", "waiting-on-author", json!({})),
            rec("a/x#5", "A/X", "done", json!({})),
            rec("a/x#6", "A/X", "requested", json!({"ai": {"status": "failed", "error": "boom"}})),
        ]});
        let out = reviews_of(&doc, None);
        let ids: Vec<&str> = out["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["a/x#1", "a/x#3", "a/x#6", "b/y#2"]);
        assert_eq!(
            (out["waitingOnAuthor"].as_u64(), out["done"].as_u64()),
            (Some(1), Some(1))
        );
        let stage = |i: usize| {
            out["requests"][i]["ai"]["stage"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(
            [stage(0), stage(1), stage(2), stage(3)],
            ["none", "stale", "failed", "ready"]
        );
        assert_eq!(out["requests"][2]["ai"]["error"], "boom");
        assert_eq!(out["requests"][3]["ai"]["counts"], json!({"attention": 1}));
        assert_eq!(out["requests"][3]["rerequest"], true);
        assert_eq!(out["requests"][0]["author"], "alice");
        assert_eq!(out["now"], 5);
    }

    #[test]
    fn a_repository_narrows_the_list_ignoring_case_and_a_document_with_nothing_is_an_empty_list() {
        let doc = json!({"records": [rec("a/x#1", "A/X", "requested", json!({})), rec("b/y#2", "B/Y", "requested", json!({}))]});
        let only = reviews_of(&doc, Some("a/x"));
        assert_eq!(only["requests"].as_array().unwrap().len(), 1);
        assert_eq!(only["requests"][0]["id"], "a/x#1");
        assert_eq!(reviews_of(&doc, Some("c/z"))["requests"], json!([]));
        assert_eq!(reviews_of(&json!({}), None)["requests"], json!([]));
    }
}
