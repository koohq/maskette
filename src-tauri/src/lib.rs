mod backend;

use backend::{Backend, BackendConfig, relay};
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // The user's data folder, in place of the old backend's `runtime/`.
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let backend = Backend::start(BackendConfig {
                data_dir,
                legacy_api_url: relay::legacy_api_url_from_env(),
            })?;
            log::info!("backend listening on {}", backend.base_url());
            #[cfg(debug_assertions)]
            backend::dev_handoff::write(&backend);

            // Built here rather than in tauri.conf.json, so the backend's
            // address and token can be injected before the UI loads.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Maskette")
                .inner_size(1280.0, 720.0)
                .min_inner_size(1024.0, 640.0)
                .center()
                .initialization_script(backend.init_script())
                .build()?;

            app.manage(backend);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app, event| {
        if let RunEvent::Exit = event {
            #[cfg(debug_assertions)]
            backend::dev_handoff::remove();
        }
    });
}
