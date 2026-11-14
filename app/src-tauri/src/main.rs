//! A native macOS window around the agent-adjutant board.
//!
//! The board is the resident server's own page, loaded into the window straight over http: its
//! token and its Origin checks only hold for the server's own origin. The app finds (or starts)
//! the server, keeps the window on it, and adds what a browser tab cannot: a translucent window
//! (with or without the macOS material behind it), native notifications, a Dock badge and a global shortcut.

// No console window for a release build on every platform that has one.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adj;
mod settings;

use std::{
    net::{SocketAddr, TcpStream},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use settings::Background;
use tauri::{
    AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu},
    webview::{NewWindowResponse, PageLoadEvent},
    window::{Color, Effect, EffectState, EffectsBuilder},
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

const MAIN: &str = "main";
/// How long the board's page gets to finish loading before the window is sent to it again.
const PAGE_LOAD_WAIT: Duration = Duration::from_secs(10);
const LOCAL_PAGE: &str = "tauri://localhost/index.html";
const SHELL_JS: &str = include_str!("js/shell.js");
const NOTIFICATION_JS: &str = include_str!("js/notification.js");

#[derive(Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Connecting,
    Ready,
    Error,
}

#[derive(Clone, Serialize)]
struct Status {
    state: Phase,
    message: String,
}

struct AppState {
    status: Mutex<Status>,
    /// The one port the window may show: the board server's. Anything else is the browser's.
    port: Mutex<Option<u16>>,
    appearance: Mutex<Appearance>,
    background_items: Mutex<Vec<(Background, CheckMenuItem<tauri::Wry>)>>,
    /// The page zoom the window shows the board at, 1 being 100%.
    zoom: Mutex<f64>,
    discovering: AtomicBool,
    /// Whether the board's page has finished loading since the window was last sent to it, and when it was sent.
    loaded: AtomicBool,
    navigated_at: Mutex<Instant>,
    /// Notifications shown and not yet clicked (see `MAX_WAITING`), and the next number to give.
    waiting: Mutex<Vec<(i32, Instant)>>,
    next_notification: AtomicI32,
}

/// What the window shows behind the board, and how opaque the board's surfaces stay over it.
#[derive(Clone, Copy)]
struct Appearance {
    background: Background,
    opacity: f64,
}

/// The appearance as the page reads it (shell.js and the board's display settings).
#[derive(Clone, Serialize)]
struct AppearanceView {
    background: &'static str,
    alpha: f64,
    #[serde(rename = "minAlpha")]
    min_alpha: f64,
    #[serde(rename = "maxAlpha")]
    max_alpha: f64,
}

impl Appearance {
    fn view(self) -> AppearanceView {
        AppearanceView {
            background: self.background.key(),
            alpha: self.opacity,
            min_alpha: settings::MIN_OPACITY,
            max_alpha: settings::MAX_OPACITY,
        }
    }

    fn json(self) -> String {
        serde_json::to_string(&self.view()).expect("the appearance serialises")
    }
}

fn set_status(app: &AppHandle, state: Phase, message: impl Into<String>) {
    *app.state::<AppState>().status.lock().unwrap() = Status {
        state,
        message: message.into(),
    };
}

// ── Commands: the local page (get_status, retry) and the board (the rest) ──

#[tauri::command]
fn get_status(app: AppHandle) -> Status {
    app.state::<AppState>().status.lock().unwrap().clone()
}

#[tauri::command]
fn retry(app: AppHandle) {
    connect(&app);
}

/// The notification plugin answers "granted" on desktop whatever macOS says: it does not read the
/// app's authorization in System Settings, and the plugin's backend has no call for it. So a person
/// who blocked the app there silences its notifications and, because the page still reports that
/// it rings, the server's own fallback too while the app is open.
fn permission_word(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        _ => "default",
    }
}

#[tauri::command]
fn notification_permission(app: AppHandle) -> &'static str {
    app.notification()
        .permission_state()
        .map(permission_word)
        .unwrap_or("default")
}

#[tauri::command]
fn request_notification_permission(app: AppHandle) -> Result<&'static str, String> {
    app.notification()
        .request_permission()
        .map(permission_word)
        .map_err(|e| e.to_string())
}

/// How many notifications may be waiting for a click at once, and for how long one counts as
/// waiting. With a click handler registered the plugin keeps a thread (and, on macOS, a poll of
/// Notification Center) per notification, until it is clicked or leaves Notification Center; a
/// dismissal is not reported, so the end of that thread cannot be seen. This bounds how fast they
/// can pile up: a notification that was not clicked within the time is no longer counted.
const MAX_WAITING: usize = 20;
const WAITING_FOR: Duration = Duration::from_secs(10 * 60);

/// Shows a native notification and returns its number, from a counter that lives as long as the
/// app: a click comes back with it, so the page can run that notification's `onclick`, and a
/// number from an earlier document matches nothing in a new one.
#[tauri::command]
fn notify(app: AppHandle, title: String, body: String) -> Result<i32, String> {
    let state = app.state::<AppState>();
    let id = {
        let mut waiting = state.waiting.lock().unwrap();
        waiting.retain(|(_, at)| at.elapsed() < WAITING_FOR);
        if waiting.len() >= MAX_WAITING {
            return Err("too many notifications are waiting".into());
        }
        let id = state.next_notification.fetch_add(1, Ordering::SeqCst);
        waiting.push((id, Instant::now()));
        id
    };
    app.notification()
        .builder()
        .id(id)
        .title(title)
        .body(body)
        .show()
        .map_err(|e| {
            state.waiting.lock().unwrap().retain(|(n, _)| *n != id);
            e.to_string()
        })?;
    Ok(id)
}

/// A click on a native notification: the window comes to the front even if it was hidden, and
/// the page hears which notification it was.
fn on_notification_action(app: &AppHandle, performed: &tauri_plugin_notification::ActionPerformed) {
    if performed.action_id() != "tap" {
        return;
    }
    if let Some(notification) = performed.notification() {
        let id = notification.id();
        app.state::<AppState>()
            .waiting
            .lock()
            .unwrap()
            .retain(|(n, _)| *n != id);
    }
    show_main(app);
    if let (Some(window), Some(notification)) =
        (app.get_webview_window(MAIN), performed.notification())
    {
        let _ = window.eval(format!(
            "window.__adjNotificationClicked && window.__adjNotificationClicked({})",
            notification.id()
        ));
    }
}

#[tauri::command]
fn focus_window(app: AppHandle) {
    show_main(&app);
}

/// Cmd++ from the board's page (shell.js): a menu accelerator names a physical key, so it cannot
/// say "the key that types +" on every keyboard layout, and the page catches that one instead.
#[tauri::command]
fn zoom(app: AppHandle, direction: String) -> Result<(), String> {
    let change = Zoom::from_key(&direction).ok_or("unknown zoom direction")?;
    change_zoom(&app, change);
    Ok(())
}

/// The board's display settings (app-appearance.js): either half of the appearance, or both. The
/// slider sends every step as it moves with `persist` false, and the last one with it true, so the
/// file is written once per drag. A background is always saved. A save that fails is reported to
/// the page, which shows it; what the window shows has changed all the same.
#[tauri::command]
fn set_appearance(
    app: AppHandle,
    background: Option<String>,
    opacity: Option<f64>,
    persist: bool,
) -> Result<AppearanceView, String> {
    let background = background
        .map(|key| Background::from_key(&key).ok_or("unknown background"))
        .transpose()?;
    change_appearance(&app, background, opacity, persist).map(Appearance::view)
}

// ── Window ──

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let showing = window.is_visible().unwrap_or(false)
        && window.is_focused().unwrap_or(false)
        && !window.is_minimized().unwrap_or(false);
    if showing {
        let _ = window.hide();
    } else {
        show_main(app);
    }
}

fn shell_script(appearance: Appearance) -> String {
    SHELL_JS.replace("__APPEARANCE__", &appearance.json())
}

/// The window effect behind the page: the material blurs what is behind the window, and with no
/// effect the transparent window shows it as it is. Switched on the open window, no new one needed.
fn effects(background: Background) -> Option<tauri::utils::config::WindowEffectsConfig> {
    match background {
        Background::Material => Some(
            EffectsBuilder::new()
                .effect(Effect::UnderWindowBackground)
                .state(EffectState::Active)
                .build(),
        ),
        Background::Clear => None,
    }
}

fn apply_background(window: &WebviewWindow, background: Background) {
    if let Err(e) = window.set_effects(effects(background)) {
        eprintln!("background: {e}");
    }
}

/// Puts the appearance on the page that is showing (a page that is not the board ignores it).
fn apply_appearance(window: &WebviewWindow, appearance: Appearance) {
    let _ = window.eval(format!(
        "window.__adjShell && window.__adjShell({})",
        appearance.json()
    ));
}

/// Changes the background, the opacity or both, from the menu or the board, and shows it: the
/// window's effect (when the background changed), the menu's check marks and the page. The lock is
/// held throughout, as in `change_zoom`, so that two changes at once save and show in one order.
/// A background is saved even when it is the one already shown, so that choosing it again retries
/// a save that failed. The change is shown whether or not it could be saved, and a failed save
/// comes back as the error.
fn change_appearance(
    app: &AppHandle,
    background: Option<Background>,
    opacity: Option<f64>,
    persist: bool,
) -> Result<Appearance, String> {
    let state = app.state::<AppState>();
    let mut appearance = state.appearance.lock().unwrap();
    let window = app.get_webview_window(MAIN);
    let mut saved = Ok(());
    if let Some(background) = background {
        if background != appearance.background {
            appearance.background = background;
            for (b, item) in state.background_items.lock().unwrap().iter() {
                let _ = item.set_checked(*b == background);
            }
            if let Some(window) = &window {
                apply_background(window, background);
            }
        }
        saved = settings::save_background(app, background);
    }
    if let Some(opacity) = opacity {
        appearance.opacity = settings::clamp_opacity(opacity);
        if persist && saved.is_ok() {
            saved = settings::save_opacity(app, appearance.opacity);
        }
    }
    if let Some(window) = &window {
        apply_appearance(window, *appearance);
    }
    if let Err(e) = &saved {
        eprintln!("settings: {e}");
    }
    saved.map(|()| *appearance)
}

/// Puts the page zoom on the window. WebKit keeps it on the view across loads, and it is put on
/// again when a page loads all the same, so that what the window shows is always the saved zoom.
fn apply_zoom(window: &WebviewWindow, zoom: f64) {
    if let Err(e) = window.set_zoom(zoom) {
        eprintln!("zoom: {e}");
    }
}

#[derive(Clone, Copy)]
enum Zoom {
    In,
    Out,
    Reset,
}

impl Zoom {
    fn from_key(key: &str) -> Option<Zoom> {
        match key {
            "in" => Some(Zoom::In),
            "out" => Some(Zoom::Out),
            "reset" => Some(Zoom::Reset),
            _ => None,
        }
    }

    fn apply_to(self, zoom: f64) -> f64 {
        match self {
            Zoom::In => settings::zoom_in(zoom),
            Zoom::Out => settings::zoom_out(zoom),
            Zoom::Reset => 1.0,
        }
    }
}

/// Moves the zoom one step (or back to 100%), saves it and puts it on the window. The lock is held
/// throughout, so that a menu click and a key from the page at once save and show in one order.
fn change_zoom(app: &AppHandle, change: Zoom) {
    let state = app.state::<AppState>();
    let mut zoom = state.zoom.lock().unwrap();
    *zoom = change.apply_to(*zoom);
    if let Err(e) = settings::save_zoom(app, *zoom) {
        eprintln!("settings: {e}");
    }
    if let Some(window) = app.get_webview_window(MAIN) {
        apply_zoom(&window, *zoom);
    }
}

/// The board's own URL, or one of the app's local pages: where the window may stay. Everything
/// else goes to the default browser.
fn allowed_in_window(app: &AppHandle, url: &Url) -> bool {
    let port = *app.state::<AppState>().port.lock().unwrap();
    let is_board = |u: &Url| {
        u.host_str() == Some("127.0.0.1") && port.is_some() && u.port_or_known_default() == port
    };
    match url.scheme() {
        // The app's own loading and error page, and nothing else under its scheme (not about:blank either).
        "tauri" => url.host_str() == Some("localhost") && url.path() == "/index.html",
        "http" => is_board(url),
        // The board shows a report in a frame loaded from a blob the page made itself (gate-report.js).
        "blob" => {
            Url::parse(url.path()).is_ok_and(|inner| inner.scheme() == "http" && is_board(&inner))
        }
        _ => false,
    }
}

fn open_externally(app: &AppHandle, url: &Url) {
    // Only what a browser would open: not file:, not a custom scheme the page made up.
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}

/// The count in a title such as "(3) adj": how many items wait on the person.
fn badge_count(title: &str) -> Option<i64> {
    let rest = title.strip_prefix('(')?;
    let (digits, _) = rest.split_once(") ")?;
    digits.parse::<i64>().ok().filter(|n| *n > 0)
}

fn create_window(app: &AppHandle, appearance: Appearance, zoom: f64) -> tauri::Result<()> {
    let notification = NOTIFICATION_JS.replace(
        "__PERMISSION__",
        app.notification()
            .permission_state()
            .map(permission_word)
            .unwrap_or("default"),
    );
    let script = format!("{}\n{}", shell_script(appearance), notification);

    let on_navigation = app.clone();
    let on_new_window = app.clone();
    let mut builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Adjutant")
        .inner_size(1440.0, 900.0)
        .min_inner_size(900.0, 600.0)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0));
    if let Some(effects) = effects(appearance.background) {
        builder = builder.effects(effects);
    }
    let window = builder
        // A board in a window nobody is looking at must go on reading the server's events.
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        .initialization_script(script)
        .on_navigation(move |url| {
            if allowed_in_window(&on_navigation, url) {
                true
            } else {
                open_externally(&on_navigation, url);
                false
            }
        })
        .on_new_window(move |url, _features| {
            open_externally(&on_new_window, &url);
            NewWindowResponse::Deny
        })
        .on_document_title_changed(|window, title| {
            let _ = window.set_badge_count(badge_count(&title));
        })
        .build()?;
    apply_zoom(&window, zoom);
    Ok(())
}

// ── Server connection ──

/// Finds (or starts) the server off the main thread and moves the window to it; on failure the
/// window shows the local page with the reason.
fn connect(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.discovering.swap(true, Ordering::SeqCst) {
        return;
    }
    set_status(app, Phase::Connecting, "");
    let app = app.clone();
    thread::spawn(move || {
        let configured = settings::load(&app).adj_path;
        let result = adj::discover(configured.as_deref()).and_then(|url| {
            Url::parse(&url)
                .map_err(|e| format!("サーバーの URL を読めません: {url} ({e})"))
                .map(|parsed| (url, parsed))
        });
        match result {
            Ok((_, url)) => {
                *app.state::<AppState>().port.lock().unwrap() = url.port_or_known_default();
                set_status(&app, Phase::Ready, "");
                let state = app.state::<AppState>();
                state.loaded.store(false, Ordering::SeqCst);
                *state.navigated_at.lock().unwrap() = Instant::now();
                if let Some(window) = app.get_webview_window(MAIN) {
                    let _ = window.navigate(url);
                }
            }
            Err(message) => {
                *app.state::<AppState>().port.lock().unwrap() = None;
                set_status(&app, Phase::Error, message);
                if let Some(window) = app.get_webview_window(MAIN) {
                    // `navigate` to the page the window may already be on reloads it, which is what is wanted.
                    let _ = window.navigate(LOCAL_PAGE.parse().expect("a valid URL"));
                }
            }
        }
        app.state::<AppState>()
            .discovering
            .store(false, Ordering::SeqCst);
    });
}

/// A page that failed to load shows nothing to hook, so the server is watched instead: when it
/// stops answering for a while (it was restarted, maybe on another port), look for it again
/// before showing an error.
fn watch_server(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let mut misses = 0;
        let mut unloaded = 0;
        loop {
            thread::sleep(Duration::from_secs(4));
            let state = app.state::<AppState>();
            let port = *state.port.lock().unwrap();
            let ready = state.status.lock().unwrap().state == Phase::Ready;
            let (true, Some(port)) = (ready, port) else {
                misses = 0;
                continue;
            };
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            if TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok() {
                misses = 0;
                // The port answers, but the page may still not have come up (the server restarted
                // between finding it and loading it): send the window again, and give up with the
                // error page after a few tries.
                if state.loaded.load(Ordering::SeqCst) {
                    unloaded = 0;
                } else if state.navigated_at.lock().unwrap().elapsed() > PAGE_LOAD_WAIT {
                    unloaded += 1;
                    if unloaded >= 3 {
                        unloaded = 0;
                        *state.port.lock().unwrap() = None;
                        set_status(&app, Phase::Error, "ボードのページを読み込めませんでした");
                        if let Some(window) = app.get_webview_window(MAIN) {
                            let _ = window.navigate(LOCAL_PAGE.parse().expect("a valid URL"));
                        }
                    } else {
                        connect(&app);
                    }
                }
                continue;
            }
            misses += 1;
            // A restart (stop, then start) is a gap of a few seconds: not worth starting a second server in.
            if misses >= 3 {
                misses = 0;
                connect(&app);
            }
        }
    });
}

// ── Menu ──

fn build_menu(app: &AppHandle, background: Background) -> tauri::Result<Menu<tauri::Wry>> {
    // The default menu keeps App / Edit / Window, so that Cmd+C, V, X, A and Z reach the page.
    let menu = Menu::default(app)?;
    // Only the background: the opacity is a slider in the board's display settings.
    let items = Background::ALL
        .into_iter()
        .map(|b| {
            CheckMenuItem::with_id(
                app,
                format!("background:{}", b.key()),
                b.label(),
                true,
                b == background,
                None::<&str>,
            )
            .map(|item| (b, item))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let submenu = Submenu::with_id(app, "background", "背景", true)?;
    for (_, item) in &items {
        submenu.append(item)?;
    }
    *app.state::<AppState>().background_items.lock().unwrap() = items;

    // As in a browser. Cmd++ (the key that types +) is caught by the page: see the `zoom` command.
    let zoom_items = [
        MenuItem::with_id(app, "zoom:in", "拡大", true, Some("CmdOrCtrl+="))?,
        MenuItem::with_id(app, "zoom:out", "縮小", true, Some("CmdOrCtrl+-"))?,
        MenuItem::with_id(app, "zoom:reset", "実際のサイズ", true, Some("CmdOrCtrl+0"))?,
    ];
    let separator = PredefinedMenuItem::separator(app)?;
    let append_to = |view: &Submenu<tauri::Wry>| -> tauri::Result<()> {
        for item in &zoom_items {
            view.append(item)?;
        }
        view.append(&separator)?;
        view.append(&submenu)
    };

    let view = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(s) if s.text().is_ok_and(|t| t == "View") => Some(s),
        _ => None,
    });
    match view {
        Some(view) => {
            view.set_text("表示")?;
            // Below what the default menu already has there (full screen), apart from it.
            view.append(&PredefinedMenuItem::separator(app)?)?;
            append_to(&view)?;
        }
        None => {
            let view = Submenu::with_id(app, "view", "表示", true)?;
            append_to(&view)?;
            menu.append(&view)?;
        }
    }
    Ok(menu)
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    if let Some(change) = event
        .id()
        .as_ref()
        .strip_prefix("zoom:")
        .and_then(Zoom::from_key)
    {
        change_zoom(app, change);
        return;
    }
    let Some(background) = event
        .id()
        .as_ref()
        .strip_prefix("background:")
        .and_then(Background::from_key)
    else {
        return;
    };
    // A check item flips its own mark when clicked: clicking the one already chosen would clear
    // it, so the marks are put back even when nothing else changes.
    for (b, item) in app
        .state::<AppState>()
        .background_items
        .lock()
        .unwrap()
        .iter()
    {
        let _ = item.set_checked(*b == background);
    }
    // A failed save is already logged, and the menu has nowhere to show it.
    let _ = change_appearance(app, Some(background), None, true);
}

fn register_shortcut(app: &AppHandle, shortcut: Option<&str>) {
    // None is the person's own choice to have none.
    let Some(shortcut) = shortcut else { return };
    // Already taken by another app, or misspelt in the settings: the default stands in for a bad
    // value, and without it too the app still works.
    let Err(e) = app.global_shortcut().register(shortcut) else {
        return;
    };
    eprintln!("shortcut {shortcut}: {e}");
    if shortcut != settings::DEFAULT_SHORTCUT
        && let Err(e) = app.global_shortcut().register(settings::DEFAULT_SHORTCUT)
    {
        eprintln!("shortcut {}: {e}", settings::DEFAULT_SHORTCUT);
    }
}

fn main() {
    let app = tauri::Builder::default()
        // First, so that a second launch is turned back before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_main(app);
                    }
                })
                .build(),
        )
        .plugin(
            // Not VISIBLE: a window hidden at quit must not come back hidden.
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    StateFlags::SIZE
                        | StateFlags::POSITION
                        | StateFlags::MAXIMIZED
                        | StateFlags::FULLSCREEN,
                )
                .build(),
        )
        // Not its click interception: it would send `target="_blank"` links to a command the board
        // is not granted, and they would do nothing. Left alone they reach on_new_window below.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            get_status,
            retry,
            notify,
            notification_permission,
            request_notification_permission,
            focus_window,
            zoom,
            set_appearance
        ])
        .on_menu_event(on_menu_event)
        // The page's process died (memory pressure, a crash): nothing is loaded any more, and the
        // window shows blank. Find the server and load the page again.
        .on_web_content_process_terminate(|webview| {
            let app = webview.app_handle();
            app.state::<AppState>()
                .loaded
                .store(false, Ordering::SeqCst);
            connect(app);
        })
        .on_page_load(|webview, payload| {
            // The page's load, for watch_server. Started is the main frame committing: from then
            // on the board's document is this new one, so it is unloaded until Finished, which a
            // failed load never reports. (A load that fails before committing leaves the previous
            // board page in place, which goes on polling the server by itself.) Only the main frame
            // reports either.
            let board_page = payload.url().scheme() == "http"
                && allowed_in_window(webview.app_handle(), payload.url());
            if board_page {
                let state = webview.app_handle().state::<AppState>();
                match payload.event() {
                    PageLoadEvent::Started => {
                        state.loaded.store(false, Ordering::SeqCst);
                        *state.navigated_at.lock().unwrap() = Instant::now();
                    }
                    PageLoadEvent::Finished => state.loaded.store(true, Ordering::SeqCst),
                }
            }
            // Both ends of the load: the page's own script may not be in yet at the start.
            if matches!(
                payload.event(),
                PageLoadEvent::Started | PageLoadEvent::Finished
            ) {
                let appearance = *webview
                    .app_handle()
                    .state::<AppState>()
                    .appearance
                    .lock()
                    .unwrap();
                let zoom = *webview
                    .app_handle()
                    .state::<AppState>()
                    .zoom
                    .lock()
                    .unwrap();
                if let Some(window) = webview.app_handle().get_webview_window(webview.label()) {
                    apply_appearance(&window, appearance);
                    apply_zoom(&window, zoom);
                }
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let loaded = settings::load(&handle);
            app.manage(AppState {
                status: Mutex::new(Status {
                    state: Phase::Connecting,
                    message: String::new(),
                }),
                port: Mutex::new(None),
                appearance: Mutex::new(Appearance {
                    background: loaded.background,
                    opacity: loaded.opacity,
                }),
                background_items: Mutex::new(vec![]),
                zoom: Mutex::new(loaded.zoom),
                discovering: AtomicBool::new(false),
                loaded: AtomicBool::new(false),
                navigated_at: Mutex::new(Instant::now()),
                waiting: Mutex::new(vec![]),
                next_notification: AtomicI32::new(1),
            });
            // Before any notification is shown: only those shown after a handler exists report clicks.
            let for_clicks = handle.clone();
            handle
                .notification()
                .on_action(move |performed| on_notification_action(&for_clicks, performed))?;
            handle.set_menu(build_menu(&handle, loaded.background)?)?;
            create_window(
                &handle,
                Appearance {
                    background: loaded.background,
                    opacity: loaded.opacity,
                },
                loaded.zoom,
            )?;
            register_shortcut(&handle, loaded.shortcut.as_deref());
            connect(&handle);
            watch_server(&handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the app");

    app.run(|app, event| match event {
        // The red button hides the window; the app (and its shortcut and Dock badge) stays until Cmd+Q.
        RunEvent::WindowEvent {
            event: WindowEvent::CloseRequested { api, .. },
            label,
            ..
        } if label == MAIN => {
            api.prevent_close();
            if let Some(window) = app.get_webview_window(MAIN) {
                let _ = window.hide();
            }
        }
        // A click on the Dock icon, or on a notification, activates the app.
        RunEvent::Reopen { .. } => show_main(app),
        _ => {}
    });
}
