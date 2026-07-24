//! Menu-bar / system-tray mode (M5).
//!
//! Rust owns the tray and the window lifecycle: closing the window hides it
//! (the engine keeps running untouched); the tray menu shows the current
//! song, mirrors the automation master toggle, and offers open/quit.

// ---------------------------------------------------------------------------
// Pure menu model (unit-tested; no tauri types)
// ---------------------------------------------------------------------------

/// Longest now-playing line shown in the tray menu, in characters — native
/// menus get unwieldy beyond that.
pub const NOW_PLAYING_MAX_CHARS: usize = 40;

/// Menu line while nothing displayable is playing.
pub const NOTHING_PLAYING: &str = "Nothing playing";

/// What the tray menu should display; computed from playback + toggle state
/// so both update paths (poll events, toggle changes) share one mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuModel {
    pub now_playing: String,
    pub automation_checked: bool,
}

/// Map playback + master-toggle state to the tray menu contents.
pub fn menu_model(track: Option<(&str, &[String])>, automation_on: bool) -> TrayMenuModel {
    TrayMenuModel {
        now_playing: now_playing_line(track),
        automation_checked: automation_on,
    }
}

/// "Title — Artist, Artist" truncated to [`NOW_PLAYING_MAX_CHARS`], or
/// [`NOTHING_PLAYING`]. Truncation counts characters (not bytes), so
/// multi-byte titles never split a code point.
pub fn now_playing_line(track: Option<(&str, &[String])>) -> String {
    let Some((title, artists)) = track else {
        return NOTHING_PLAYING.to_owned();
    };
    let line = if artists.is_empty() {
        title.to_owned()
    } else {
        format!("{title} — {}", artists.join(", "))
    };
    if line.chars().count() <= NOW_PLAYING_MAX_CHARS {
        return line;
    }
    let mut truncated: String = line.chars().take(NOW_PLAYING_MAX_CHARS - 1).collect();
    truncated.push('…');
    truncated
}

// ---------------------------------------------------------------------------
// Tray runtime (icon, native menu, window lifecycle)
// ---------------------------------------------------------------------------

use std::sync::atomic::Ordering;
use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::commands::AppState;
use crate::player::TrackInfo;
use crate::{commands, player};

/// Label of the app's single window (matches tauri.conf.json / capabilities).
const MAIN_WINDOW: &str = "main";

const NOW_PLAYING_ID: &str = "tray-now-playing";
const AUTOMATION_ID: &str = "tray-automation";
const OPEN_ID: &str = "tray-open";
const QUIT_ID: &str = "tray-quit";

/// Handles to the mutable tray menu items, managed as Tauri state so the
/// poll loop (now-playing line) and the toggle path (checkbox) can update
/// them from anywhere.
pub struct TrayHandles {
    now_playing: MenuItem<Wry>,
    automation: CheckMenuItem<Wry>,
    /// Last text written to the now-playing item — skips no-op menu rewrites
    /// (heartbeat events repeat unchanged snapshots).
    last_line: Mutex<String>,
}

/// Build the tray icon + menu and manage the item handles. Called once in
/// setup, after the persisted automation toggle has been applied.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let automation_on = app.state::<AppState>().automation_on.load(Ordering::SeqCst);
    let model = menu_model(None, automation_on);

    let now_playing = MenuItemBuilder::with_id(NOW_PLAYING_ID, &model.now_playing)
        .enabled(false)
        .build(app)?;
    let automation = CheckMenuItemBuilder::with_id(AUTOMATION_ID, "Automation")
        .checked(model.automation_checked)
        .build(app)?;
    let open = MenuItemBuilder::with_id(OPEN_ID, "Open Cued").build(app)?;
    let quit = MenuItemBuilder::with_id(QUIT_ID, "Quit Cued").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&now_playing, &automation])
        .separator()
        .items(&[&open, &quit])
        .build()?;

    let builder = TrayIconBuilder::with_id("cued-tray")
        .menu(&menu)
        .tooltip("Cued")
        .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| on_tray_icon_event(tray.app_handle(), &event));
    // macOS: monochrome template image (adapts to the menu bar theme), menu
    // on left click as the platform expects. Windows: the .ico, menu stays
    // on right click so a left click can open the window instead.
    #[cfg(target_os = "macos")]
    let builder = builder
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/tray/tray-template@2x.png"
        ))?)
        .icon_as_template(true)
        .show_menu_on_left_click(true);
    #[cfg(not(target_os = "macos"))]
    let builder = builder
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/icon.ico"
        ))?)
        .show_menu_on_left_click(false);
    builder.build(app)?;

    app.manage(TrayHandles {
        now_playing,
        automation,
        last_line: Mutex::new(model.now_playing),
    });
    Ok(())
}

fn on_menu_event(app: &AppHandle, id: &str) {
    // Every menu interaction doubles as a wake nudge (cheap, idempotent).
    nudge_engine(app);
    match id {
        OPEN_ID => show_main_window(app),
        QUIT_ID => quit_app(app),
        AUTOMATION_ID => toggle_automation_from_menu(app),
        _ => {}
    }
}

fn on_tray_icon_event(app: &AppHandle, event: &TrayIconEvent) {
    // Clicks wake the engine; Enter/Move/Leave hover noise does not.
    let is_click = matches!(
        event,
        TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. }
    );
    if !is_click {
        return;
    }
    nudge_engine(app);
    // Windows: a plain left click opens the window (the menu is on right
    // click there); on macOS the left click already opens the menu.
    #[cfg(not(target_os = "macos"))]
    if let TrayIconEvent::Click {
        button: tauri::tray::MouseButton::Left,
        button_state: tauri::tray::MouseButtonState::Up,
        ..
    } = event
    {
        show_main_window(app);
    }
}

/// The native checkbox has already flipped when this runs; persist the new
/// value through the shared toggle path and revert the checkbox if that fails.
fn toggle_automation_from_menu(app: &AppHandle) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let next = match handles.automation.is_checked() {
        Ok(checked) => checked,
        Err(e) => {
            eprintln!("cued: cannot read the tray automation checkbox: {e}");
            return;
        }
    };
    if let Err(e) = commands::apply_automation_enabled(app, next) {
        eprintln!("cued: could not save the automation toggle from the tray: {e}");
        if let Err(e) = handles.automation.set_checked(!next) {
            eprintln!("cued: cannot revert the tray automation checkbox: {e}");
        }
    }
}

/// Idempotent engine nudge: restart the loop if none is running (self-heal)
/// and cut the current sleep short so tray interactions see fresh state.
fn nudge_engine(app: &AppHandle) {
    player::start(app);
    app.state::<AppState>().player.wake();
}

/// Show + focus the main window and restore the Dock presence on macOS.
/// Used by the tray menu, macOS Reopen and the single-instance callback.
pub fn show_main_window(app: &AppHandle) {
    // Regular first, so macOS has the Dock icon back before the window fronts.
    #[cfg(target_os = "macos")]
    if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
        eprintln!("cued: cannot restore the Dock presence: {e}");
    }
    match app.get_webview_window(MAIN_WINDOW) {
        Some(window) => {
            for (what, result) in [
                ("show", window.show()),
                ("unminimize", window.unminimize()),
                ("focus", window.set_focus()),
            ] {
                if let Err(e) = result {
                    eprintln!("cued: cannot {what} the main window: {e}");
                }
            }
        }
        None => eprintln!("cued: main window not found"),
    }
    app.state::<AppState>()
        .window_visible
        .store(true, Ordering::SeqCst);
    nudge_engine(app);
}

/// Hide the window into the tray (the engine is untouched); on macOS the
/// Dock icon disappears with it.
pub fn hide_to_tray(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        if let Err(e) = window.hide() {
            eprintln!("cued: cannot hide the main window: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Accessory) {
        eprintln!("cued: cannot drop the Dock presence: {e}");
    }
    let state = app.state::<AppState>();
    state.window_visible.store(false, Ordering::SeqCst);
    // A loop parked in the visible suspend (waiting for a UI wake that will
    // never come now) must re-decide and switch to the hidden slow poll.
    state.player.wake();
}

/// Exit for real: invalidate the engine loop's generation first so no
/// request is issued during teardown, then end the process.
fn quit_app(app: &AppHandle) {
    app.state::<AppState>().player.stop();
    app.exit(0);
}

/// Refresh the now-playing menu line; called by the poll loop only on
/// meaningful (changed-only) emissions, and skipped when the text is
/// unchanged anyway (heartbeats).
pub fn update_now_playing(app: &AppHandle, track: Option<&TrackInfo>) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let line = now_playing_line(track.map(|t| (t.title.as_str(), t.artists.as_slice())));
    let Ok(mut last) = handles.last_line.lock() else {
        // Poisoned = an earlier update panicked; skip rather than crash.
        return;
    };
    if *last == line {
        return;
    }
    if let Err(e) = handles.now_playing.set_text(&line) {
        eprintln!("cued: cannot update the tray now-playing line: {e}");
        return;
    }
    *last = line;
}

/// Mirror the master toggle into the tray checkbox (no-op before tray init).
pub fn sync_automation(app: &AppHandle, enabled: bool) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    if let Err(e) = handles.automation.set_checked(enabled) {
        eprintln!("cued: cannot update the tray automation checkbox: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artists(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    // -- now_playing_line ---------------------------------------------------

    #[test]
    fn nothing_playing_when_no_track() {
        assert_eq!(now_playing_line(None), NOTHING_PLAYING);
    }

    #[test]
    fn joins_title_and_artists() {
        let a = artists(&["A", "B"]);
        assert_eq!(now_playing_line(Some(("Song", &a))), "Song — A, B");
    }

    #[test]
    fn title_without_artists_stands_alone() {
        assert_eq!(now_playing_line(Some(("Song", &[]))), "Song");
    }

    #[test]
    fn short_lines_are_untouched() {
        let a = artists(&["Artist"]);
        let line = now_playing_line(Some(("Title", &a)));
        assert_eq!(line, "Title — Artist");
        assert!(line.chars().count() <= NOW_PLAYING_MAX_CHARS);
    }

    #[test]
    fn long_lines_truncate_to_the_cap_with_ellipsis() {
        let a = artists(&["Someone With A Rather Long Name"]);
        let line = now_playing_line(Some(("An Extremely Long Song Title Indeed", &a)));
        assert_eq!(line.chars().count(), NOW_PLAYING_MAX_CHARS);
        assert!(line.ends_with('…'), "truncated line must end in …: {line}");
    }

    #[test]
    fn truncation_never_splits_a_multibyte_char() {
        // 50 umlauts: every byte boundary inside a char would panic a naive
        // byte slice; chars-based truncation must stay valid UTF-8.
        let title = "ü".repeat(50);
        let line = now_playing_line(Some((title.as_str(), &[])));
        assert_eq!(line.chars().count(), NOW_PLAYING_MAX_CHARS);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn exactly_at_the_cap_is_not_truncated() {
        let title = "x".repeat(NOW_PLAYING_MAX_CHARS);
        let line = now_playing_line(Some((title.as_str(), &[])));
        assert_eq!(line, title);
    }

    // -- menu_model (toggle-sync state mapping) ------------------------------

    #[test]
    fn menu_check_mirrors_the_toggle_in_both_states() {
        assert!(menu_model(None, true).automation_checked);
        assert!(!menu_model(None, false).automation_checked);
    }

    #[test]
    fn model_carries_the_now_playing_line() {
        let a = artists(&["A"]);
        let m = menu_model(Some(("Song", &a)), true);
        assert_eq!(m.now_playing, "Song — A");
        assert_eq!(menu_model(None, true).now_playing, NOTHING_PLAYING);
    }
}
