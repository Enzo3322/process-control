mod config;
mod models;
mod runtime_ops;
mod state;
mod tray_app;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let snapshot = config::load_config_snapshot(&app.handle())?;
            app.manage(AppState::new(snapshot));

            tray_app::create_tray(&app.handle())?;
            tray_app::start_config_watcher(app.handle().clone())?;
            tray_app::start_process_refresh_loop(app.handle().clone());

            if let Err(error) = tray_app::refresh_tray(&app.handle(), None) {
                tray_app::refresh_tray_with_message(&app.handle(), &error);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
