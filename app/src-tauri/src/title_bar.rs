//! The window's title bar. The board runs under it (an overlay title bar with the title hidden), so
//! the page draws its own drag strip (shell.js) and asks the app what a double-click on it does.

use std::process::Command;

/// What a double-click on the title bar does, as System Settings says (Desktop & Dock, "Double-click
/// a window's title bar to").
#[derive(Debug, PartialEq, Eq)]
pub enum DoubleClick {
    /// Zoom or Fill: the window grows to the screen, or back.
    Zoom,
    Minimize,
    Nothing,
}

/// Reads the setting from the global domain. `AppleActionOnDoubleClick` is the current key; a system
/// that never wrote it may still have the older `AppleMiniaturizeOnDoubleClick`.
pub fn double_click() -> DoubleClick {
    let action = read_global("AppleActionOnDoubleClick");
    let miniaturize = match action {
        Some(_) => None,
        None => read_global("AppleMiniaturizeOnDoubleClick"),
    };
    double_click_from(action.as_deref(), miniaturize.as_deref())
}

/// What the two keys mean together. Fill and Zoom both grow the window here: the window can be
/// maximized, not tiled. Anything unknown, or nothing at all, is the system's default, Zoom.
pub fn double_click_from(action: Option<&str>, miniaturize: Option<&str>) -> DoubleClick {
    match action.map(str::trim) {
        Some("Minimize") => DoubleClick::Minimize,
        Some("None") => DoubleClick::Nothing,
        Some(_) => DoubleClick::Zoom,
        None => match miniaturize.map(str::trim) {
            Some("1") => DoubleClick::Minimize,
            _ => DoubleClick::Zoom,
        },
    }
}

/// A key of the global defaults domain, or None when it is not set (or `defaults` cannot be run).
fn read_global(key: &str) -> Option<String> {
    let output = Command::new("/usr/bin/defaults")
        .args(["read", "-g", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_key_decides() {
        assert_eq!(
            double_click_from(Some("Minimize"), None),
            DoubleClick::Minimize
        );
        assert_eq!(double_click_from(Some("None"), None), DoubleClick::Nothing);
        assert_eq!(double_click_from(Some("Maximize"), None), DoubleClick::Zoom);
        assert_eq!(double_click_from(Some("Fill"), None), DoubleClick::Zoom);
        assert_eq!(
            double_click_from(Some("Minimize\n"), None),
            DoubleClick::Minimize
        );
    }

    #[test]
    fn the_older_key_counts_only_without_the_current_one() {
        assert_eq!(double_click_from(None, Some("1")), DoubleClick::Minimize);
        assert_eq!(double_click_from(None, Some("0")), DoubleClick::Zoom);
        assert_eq!(
            double_click_from(Some("Maximize"), Some("1")),
            DoubleClick::Zoom
        );
    }

    #[test]
    fn nothing_set_is_zoom() {
        assert_eq!(double_click_from(None, None), DoubleClick::Zoom);
        assert_eq!(double_click_from(Some("Unknown"), None), DoubleClick::Zoom);
    }
}
