use super::*;
use crate::registry::store::notify_clients_dir;

fn put(root: &Path, name: &str, page_at: i64, app_at: i64) {
    write_json(
        &notify_clients_dir(root).join(name),
        &json!({ "pid": 1, "pageAt": page_at, "appAt": app_at }),
    )
    .unwrap();
}

fn put_waits(root: &Path, name: &str, waits: (i64, i64)) {
    let path = notify_clients_dir(root).join(name);
    let mut record = read_json(&path).unwrap();
    record["pageWaitsAt"] = json!(waits.0);
    record["appWaitsAt"] = json!(waits.1);
    write_json(&path, &record).unwrap();
}

#[test]
fn the_app_beats_a_page_and_a_page_beats_nobody() {
    assert_eq!(route(100, 100, 150), Route::App);
    assert_eq!(route(100, 0, 150), Route::Page);
    // The app lapsed, the page did not.
    assert_eq!(route(200, 100, 191), Route::Page);
    assert_eq!(route(100, 100, 191), Route::None);
    assert_eq!(route(0, 0, 100), Route::None);
}

#[test]
fn a_client_is_open_for_ninety_seconds_after_it_was_seen() {
    assert!(!fresh(0, 100));
    assert!(fresh(100, 190));
    assert!(!fresh(100, 191));
}

#[test]
fn the_latest_of_every_server_process_counts() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(notify_clients(dir.path()), Clients::default());
    put(dir.path(), "11.json", 100, 0);
    put(dir.path(), "12.json", 90, 120);
    put(dir.path(), "13.json", 130, 50);
    assert_eq!(
        notify_clients(dir.path()),
        Clients {
            page_at: 130,
            app_at: 120,
            ..Clients::default()
        }
    );
}

#[test]
fn a_file_without_the_waits_stamps_reads_as_never_and_the_stamps_take_the_latest() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "11.json", 100, 100);
    assert_eq!(notify_clients(dir.path()).page_waits_at, 0);
    put(dir.path(), "12.json", 0, 0);
    put_waits(dir.path(), "12.json", (140, 150));
    put(dir.path(), "13.json", 0, 0);
    put_waits(dir.path(), "13.json", (160, 10));
    let all = notify_clients(dir.path());
    assert_eq!((all.page_waits_at, all.app_waits_at), (160, 150));
}

#[test]
fn a_file_that_cannot_be_read_is_skipped_not_taken_for_a_client() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "11.json", 100, 0);
    std::fs::write(notify_clients_dir(dir.path()).join("12.json"), "{not json").unwrap();
    std::fs::write(notify_clients_dir(dir.path()).join(".staging-1-0"), "x").unwrap();
    assert_eq!(notify_clients(dir.path()).page_at, 100);
}

#[test]
fn a_sighting_is_visible_at_once_and_written_only_when_it_has_moved() {
    let dir = tempfile::tempdir().unwrap();
    let file = own_file(dir.path());
    let read = || read_json(&file).unwrap();
    note_notify_client(dir.path(), Client::Page, true, 1000);
    assert_eq!(read()["pageAt"], 1000);
    note_notify_client(dir.path(), Client::Page, true, 1004);
    assert_eq!(read()["pageAt"], 1000, "within the throttle: not written");
    // But this process itself reads its own latest.
    assert_eq!(notify_clients(dir.path()).page_at, 1004);
    note_notify_client(dir.path(), Client::Page, true, 1010);
    assert_eq!(read()["pageAt"], 1010);
    assert_eq!(read()["appAt"], 0);
    note_notify_client(dir.path(), Client::App, false, 1011);
    assert_eq!(read()["appAt"], 1011, "a kind not yet written is written");
    assert_eq!(notify_clients(dir.path()).route(1012), Route::App);
}

#[test]
fn the_files_of_dead_servers_not_heard_from_for_a_day_are_removed() {
    let dir = tempfile::tempdir().unwrap();
    let file = |name: &str| notify_clients_dir(dir.path()).join(name);
    put(dir.path(), "11.json", 100, 0);
    put(dir.path(), "12.json", 90_000, 0);
    // Old, but its process is alive: its owner may have just rewritten it.
    put(dir.path(), "13.json", 100, 0);
    // Not a pid at all.
    put(dir.path(), "x.json", 100, 0);
    prune_old_clients_with(dir.path(), 100_000, &|pid| pid != 13);
    assert!(!file("11.json").exists());
    assert!(file("12.json").exists(), "heard from within a day");
    assert!(file("13.json").exists(), "its process is alive");
    assert!(file("x.json").exists(), "no pid to ask about");
}

#[test]
fn a_process_that_is_gone_is_told_from_one_that_is_alive() {
    assert!(!process_is_gone(std::process::id()));
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    assert!(process_is_gone(pid));
}

#[test]
fn a_client_without_the_waits_preference_is_open_but_does_not_ring_waits() {
    let dir = tempfile::tempdir().unwrap();
    note_notify_client(dir.path(), Client::App, false, 2000);
    let now = 2001;
    assert_eq!(notify_clients(dir.path()).route(now), Route::App);
    assert_eq!(notify_clients(dir.path()).app_waits_at, 0);
    assert_eq!(local_notify_clients(dir.path()).app_at, 2000);
    note_notify_client(dir.path(), Client::App, true, 2002);
    assert_eq!(local_notify_clients(dir.path()).app_waits_at, 2002);
}

#[test]
fn a_wait_goes_to_the_target_kind_and_is_queued_when_that_client_reads_another_process() {
    let c = |page_at, app_at, page_waits_at, app_waits_at| Clients {
        page_at,
        app_at,
        page_waits_at,
        app_waits_at,
    };
    let none = Clients::default();
    let now = 1000;
    // Nobody open: the notifier.
    assert_eq!(wait_route(none, none, now), WaitRoute::Notifier);
    assert_eq!(
        wait_route(c(1, 1, 1, 1), c(1, 1, 1, 1), now),
        WaitRoute::Notifier
    );
    // The app polls this process: it rings.
    let app = c(0, 990, 0, 990);
    assert_eq!(wait_route(app, app, now), WaitRoute::Leave);
    // The app is open on another process: queue for it, even if a page polls here.
    let page_here = c(995, 0, 995, 0);
    assert_eq!(
        wait_route(c(995, 990, 995, 990), page_here, now),
        WaitRoute::Queue(Client::App)
    );
    // A page on another process only: queue for the page.
    assert_eq!(
        wait_route(c(995, 0, 995, 0), none, now),
        WaitRoute::Queue(Client::Page)
    );
    // The target has 確認待ち off: nothing rings, and a page here does not take over.
    assert_eq!(
        wait_route(c(995, 990, 995, 0), page_here, now),
        WaitRoute::Drop
    );
    // The app here that cannot ring waits is not a local client for them.
    assert_eq!(
        wait_route(c(0, 990, 0, 0), c(0, 990, 0, 0), now),
        WaitRoute::Drop
    );
}

#[test]
fn concurrent_sightings_never_leave_an_older_snapshot_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let handles: Vec<_> = (0..8)
        .map(|n| {
            let root = root.clone();
            std::thread::spawn(move || {
                for step in 0..50 {
                    let now = 10_000 + step * 10;
                    let client = if (n + step) % 2 == 0 {
                        Client::Page
                    } else {
                        Client::App
                    };
                    note_notify_client(&root, client, true, now);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // Whatever the file holds is never behind what a later write had: one more sighting of each
    // kind, far enough on to be written, and the file is exactly what this process holds.
    note_notify_client(dir.path(), Client::Page, true, 20_000);
    note_notify_client(dir.path(), Client::App, true, 20_010);
    let on_disk = read_json(&own_file(dir.path())).unwrap();
    let held = local_notify_clients(dir.path());
    assert_eq!(on_disk["pageAt"], held.page_at);
    assert_eq!(on_disk["appAt"], held.app_at);
    assert_eq!(on_disk["pageWaitsAt"], held.page_waits_at);
    assert_eq!(on_disk["appWaitsAt"], held.app_waits_at);
}

#[test]
fn a_write_that_fails_is_tried_again_at_the_next_sighting() {
    let dir = tempfile::tempdir().unwrap();
    // A file where the directory has to be.
    std::fs::write(notify_clients_dir(dir.path()), "x").unwrap();
    note_notify_client(dir.path(), Client::Page, true, 1000);
    std::fs::remove_file(notify_clients_dir(dir.path())).unwrap();
    // Within the throttle of what would have been written, but nothing was: it is written now.
    note_notify_client(dir.path(), Client::Page, true, 1001);
    assert_eq!(read_json(&own_file(dir.path())).unwrap()["pageAt"], 1001);
}
