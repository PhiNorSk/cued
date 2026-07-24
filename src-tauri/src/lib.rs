mod automation;
mod commands;
mod config;
mod error;
mod heatmap;
mod insights;
mod pkce;
mod player;
mod presets;
mod server;
mod spotify;
mod suggestions;
mod token_store;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::Manager;
    tauri::Builder::default()
        // Must be registered first: a second launch focuses this instance
        // instead of starting twice.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            app.manage(commands::AppState::new()?);
            // Never aborts startup: an unopenable preset DB degrades to
            // per-command errors plus a UI notice.
            app.manage(presets::PresetDb::init(app.handle()));
            // Apply the persisted automation toggle (an unreadable config
            // falls back to the default: on).
            let automation_on = config::load_automation_enabled(app.handle()).unwrap_or(true);
            app.state::<commands::AppState>()
                .automation_on
                .store(automation_on, std::sync::atomic::Ordering::SeqCst);
            // Apply the persisted listening-insights toggle (default: on).
            let insights_on = config::load_insights_enabled(app.handle()).unwrap_or(true);
            app.state::<commands::AppState>()
                .insights_on
                .store(insights_on, std::sync::atomic::Ordering::SeqCst);
            // Start the background insights writer (after PresetDb is managed)
            // and expose its enqueue handle to the poll loop.
            app.manage(insights::spawn(app.handle()));
            // After the toggle is applied — the tray checkbox mirrors it.
            tray::init(app.handle())?;
            Ok(())
        })
        // Closing the window hides Cued into the tray; the engine runs on.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                tray::hide_to_tray(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_client_id,
            commands::save_client_id,
            commands::open_spotify_dashboard,
            commands::open_support_page,
            commands::connect_spotify,
            commands::get_auth_status,
            commands::logout,
            commands::player_wake,
            commands::save_preset,
            commands::get_preset,
            commands::list_presets,
            commands::delete_preset,
            commands::get_preset_db_health,
            commands::get_automation_enabled,
            commands::set_automation_enabled,
            commands::set_edit_mode,
            commands::ui_seek,
            commands::get_insights_enabled,
            commands::set_insights_enabled,
            commands::get_insights_count,
            commands::delete_all_insights,
            commands::get_track_heatmap,
            commands::analyze_suggestions,
            commands::get_track_suggestions,
            commands::list_suggestions,
            commands::accept_suggestion,
            commands::undo_suggestion,
            commands::dismiss_suggestion,
            commands::ignore_suggestion,
            commands::set_auto_skip_applied,
            commands::get_suggestion_toggles,
            commands::set_suggestion_toggles,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            // Safety net: should the hidden window ever be destroyed, keep
            // the process alive for tray mode. Explicit exits (tray Quit /
            // Cmd+Q) carry an exit code and pass through.
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } => api.prevent_exit(),
            // Stop the engine loop via its generation on every real exit
            // path, so no request is issued during teardown.
            tauri::RunEvent::Exit => app.state::<commands::AppState>().player.stop(),
            // macOS: activating the app (e.g. from Spotlight) reopens it.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => tray::show_main_window(app),
            _ => {}
        });
}
