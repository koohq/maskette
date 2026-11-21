mod backend;
mod updater;

use std::path::PathBuf;
use std::sync::Arc;

use backend::settings::secrets::{self, KeyringSecrets};
use backend::{Backend, BackendConfig};
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// Where Maskette keeps its data. Debug builds use `MASKETTE_DATA_DIR`
/// instead when it is set, so a first launch can be tried on an empty folder
/// without touching the real one.
fn data_dir(app: &tauri::App) -> tauri::Result<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("MASKETTE_DATA_DIR").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    app.path().app_data_dir()
}

/// The language the sample project is added in on a first launch: the
/// system's, as the UI starts in. Debug builds take `MASKETTE_SAMPLE_LOCALE`
/// (`ja`, `en` or `zh-CN`) instead when it is set, so the sample can be tried
/// in each language on one PC.
fn sample_locale() -> &'static str {
    #[cfg(debug_assertions)]
    if let Some(locale) = std::env::var("MASKETTE_SAMPLE_LOCALE")
        .ok()
        .filter(|locale| !locale.is_empty())
    {
        return backend::agent::sample::ui_locale([locale]);
    }
    backend::agent::sample::ui_locale(sys_locale::get_locales())
}

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
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            // A debug build would replace the installed app with a release.
            let updater: Option<Arc<dyn backend::update::Updater>> = if cfg!(debug_assertions) {
                None
            } else {
                Some(Arc::new(updater::TauriUpdater::new(app.handle().clone())))
            };

            // The user's data folder, in place of the old backend's `runtime/`.
            // A new one (the first launch) gets the sample project.
            let data_dir = data_dir(app)?;
            backend::agent::sample::open_data_dir(&data_dir, sample_locale())?;

            let backend = Backend::start(BackendConfig {
                data_dir,
                secrets: Arc::new(KeyringSecrets::new(secrets::SERVICE)),
                updater,
                app_version: app.package_info().version.to_string(),
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
