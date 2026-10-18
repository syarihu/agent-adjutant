use super::*;

fn seen_at(read_at: Option<Instant>) -> Seen {
    Seen {
        read_at,
        ..Seen::default()
    }
}

fn logins(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn only_a_github_login_is_asked_about() {
    assert!(valid_login("acme"));
    assert!(valid_login("a-b-9"));
    assert!(valid_login(&"a".repeat(39)));
    for bad in ["", "a/b", "..", "-acme", &"a".repeat(40), "a b", "ａ"] {
        assert!(!valid_login(bad), "{bad:?}");
    }
}

#[test]
fn an_organisation_and_a_user_are_read_and_a_plain_url_is_dropped() {
    let org = parse_owner(
        r#"{"login":"Acme","type":"Organization","avatar":"https://avatars.githubusercontent.com/u/1"}"#,
    )
    .unwrap();
    assert_eq!(org.login, "Acme");
    assert_eq!(org.kind.as_deref(), Some("organization"));
    assert_eq!(
        org.avatar_url.as_deref(),
        Some("https://avatars.githubusercontent.com/u/1")
    );
    let user = parse_owner(r#"{"login":"me","type":"User","avatar":null}"#).unwrap();
    assert_eq!(user.kind.as_deref(), Some("user"));
    assert_eq!(user.avatar_url, None);
    let plain = parse_owner(r#"{"login":"me","type":"User","avatar":"http://x/a.png"}"#).unwrap();
    assert_eq!(plain.avatar_url, None);
    assert!(parse_owner("not json").is_err());
    assert!(parse_owner(r#"{"type":"User"}"#).is_err());
}

#[test]
fn an_owner_is_due_when_never_read_or_read_a_day_ago() {
    let now = Instant::now();
    let hour = Duration::from_secs(60 * 60);
    let Some(long_ago) = now.checked_sub(25 * hour) else {
        return;
    };
    let mut seen = HashMap::new();
    assert_eq!(due(&seen, &logins(&["Acme", "acme"]), now), ["acme"]);
    seen.insert("acme".to_string(), seen_at(Some(now - hour)));
    assert!(due(&seen, &logins(&["acme"]), now).is_empty());
    seen.insert("acme".to_string(), seen_at(Some(long_ago)));
    assert_eq!(due(&seen, &logins(&["acme"]), now), ["acme"]);
}

#[test]
fn an_owner_being_read_or_waiting_out_a_failure_is_not_due() {
    let now = Instant::now();
    let mut seen = HashMap::new();
    seen.insert(
        "asking".to_string(),
        Seen {
            asking: true,
            ..Seen::default()
        },
    );
    seen.insert(
        "failed".to_string(),
        Seen {
            failures: 1,
            retry_at: Some(now + Duration::from_secs(10)),
            ..Seen::default()
        },
    );
    seen.insert(
        "retry".to_string(),
        Seen {
            failures: 1,
            retry_at: Some(now),
            ..Seen::default()
        },
    );
    assert_eq!(
        due(&seen, &logins(&["asking", "failed", "retry"]), now),
        ["retry"]
    );
}

#[test]
fn a_failure_keeps_the_icon_read_before_and_backs_off() {
    let icons = OwnerIcons::default();
    let icon = OwnerIcon {
        login: "Acme".into(),
        avatar_url: Some("https://a/x".into()),
        kind: Some("organization".into()),
    };
    let now = Instant::now();
    icons.record("acme", Ok(icon.clone()), now);
    icons.record("acme", Err("rate limited".into()), now);
    assert_eq!(icons.look(&logins(&["acme"])), [icon]);
    let seen = icons.seen.lock().unwrap();
    assert_eq!(seen["acme"].failures, 1);
    assert_eq!(seen["acme"].retry_at, Some(now + RETRY_AFTER[0]));
}

#[test]
fn an_owner_not_read_yet_is_listed_by_name() {
    let icons = OwnerIcons::default();
    assert_eq!(
        icons.look(&logins(&["acme"])),
        [OwnerIcon {
            login: "acme".into(),
            avatar_url: None,
            kind: None
        }]
    );
}

#[test]
fn the_read_asks_for_the_users_endpoint_and_a_failed_run_is_an_error() {
    let ran = |ok: bool, stdout: &str| {
        let stdout = stdout.to_string();
        move |args: &[&str], _| {
            assert_eq!(
                &args[..4],
                ["api", "--hostname", "github.com", "users/acme"]
            );
            assert_eq!(args[4], "--jq");
            Ok(GhRun {
                ok,
                stdout: stdout.clone(),
                stderr: "boom\n".to_string(),
            })
        }
    };
    let got = read_owner_with(
        "acme",
        &ran(
            true,
            r#"{"login":"acme","type":"User","avatar":"https://a/x"}"#,
        ),
    )
    .unwrap();
    assert_eq!(got.avatar_url.as_deref(), Some("https://a/x"));
    assert_eq!(read_owner_with("acme", &ran(false, "")), Err("boom".into()));
    // Nothing is run for a name that is not a login.
    assert!(read_owner_with("a/b", &|_, _| panic!("asked")).is_err());
}

#[test]
fn a_read_thread_that_ends_early_leaves_nothing_marked_as_being_read() {
    let icons = OwnerIcons::default();
    {
        let mut seen = icons.seen.lock().unwrap();
        seen.entry("acme".to_string()).or_default().asking = true;
        seen.entry("beta".to_string()).or_default().asking = true;
    }
    // `acme` was read; the thread died on the way to `beta`.
    icons.record("acme", Err("x".into()), Instant::now());
    drop(Unmark {
        icons: &icons,
        left: vec!["beta".to_string()],
    });
    let seen = icons.seen.lock().unwrap();
    assert!(!seen["acme"].asking && !seen["beta"].asking);
}
