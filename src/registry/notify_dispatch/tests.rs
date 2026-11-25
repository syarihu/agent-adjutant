use super::*;
use crate::registry::notify_clients::Client;
use crate::registry::store::notify_clients_dir;
use std::cell::RefCell;

fn clients(root: &Path, page_at: i64, app_at: i64) {
    write_json(
        &notify_clients_dir(root).join("1.json"),
        &json!({ "pid": 1, "pageAt": page_at, "appAt": app_at }),
    )
    .unwrap();
}

/// The terminal notifier, present or not, and what was run.
struct Fake {
    installed: bool,
    ran: RefCell<Vec<String>>,
}

impl Fake {
    fn new(installed: bool) -> Self {
        Fake {
            installed,
            ran: RefCell::new(Vec::new()),
        }
    }

    fn notify(&self, root: &Path, hook: &Hook, now: i64) -> Notified {
        notify_person_with(
            root,
            hook,
            "o/r",
            "adjutant / r",
            "done",
            now,
            &|t, m| self.installed.then(|| format!("tn {t} {m}")),
            &|command| {
                self.ran.borrow_mut().push(command.to_string());
                Ok(())
            },
        )
        .unwrap()
    }
}

#[test]
fn unset_goes_to_the_app_then_a_page_then_the_notifier_then_nowhere() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new(true);
    clients(dir.path(), 100, 100);
    assert_eq!(
        fake.notify(dir.path(), &Hook::BuiltIn, 150),
        Notified::Queued(Client::App)
    );
    clients(dir.path(), 140, 100);
    assert_eq!(
        fake.notify(dir.path(), &Hook::BuiltIn, 195),
        Notified::Queued(Client::Page)
    );
    assert!(fake.ran.borrow().is_empty(), "a queued notice is not run");
    assert_eq!(claim_notices(dir.path(), 9).len(), 2);
    // Both lapsed: the notifier, if it is installed.
    assert_eq!(fake.notify(dir.path(), &Hook::BuiltIn, 400), Notified::Ran);
    assert_eq!(*fake.ran.borrow(), ["tn adjutant / r done"]);
    assert!(claim_notices(dir.path(), 9).is_empty());
    let none = Fake::new(false);
    assert_eq!(
        none.notify(dir.path(), &Hook::BuiltIn, 400),
        Notified::NoNotifier
    );
    assert!(none.ran.borrow().is_empty());
}

#[test]
fn a_configured_command_and_off_never_look_at_who_is_open() {
    let dir = tempfile::tempdir().unwrap();
    clients(dir.path(), 100, 100);
    let fake = Fake::new(true);
    assert_eq!(
        fake.notify(dir.path(), &Hook::Command("say {message}".into()), 110),
        Notified::Ran
    );
    assert_eq!(*fake.ran.borrow(), ["say done"]);
    assert_eq!(fake.notify(dir.path(), &Hook::Off, 110), Notified::Off);
    assert!(claim_notices(dir.path(), 9).is_empty());
}

#[test]
fn a_dry_run_plans_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    clients(dir.path(), 100, 0);
    let plan = plan_notification_with(dir.path(), &Hook::BuiltIn, "o/r", "t", "m", 120, &|_, _| {
        None
    });
    assert_eq!(plan, Plan::Queue(Client::Page));
    assert!(claim_notices(dir.path(), 9).is_empty());
}

#[test]
fn a_notice_that_cannot_be_queued_falls_back_to_the_notifier() {
    let dir = tempfile::tempdir().unwrap();
    clients(dir.path(), 100, 0);
    // A file where the queue's directory has to be.
    std::fs::write(dir.path().join("notices"), "x").unwrap();
    let fake = Fake::new(true);
    assert_eq!(fake.notify(dir.path(), &Hook::BuiltIn, 120), Notified::Ran);
}
