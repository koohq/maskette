//! The Tauri updater plugin, behind the backend's update routes
//! ([`crate::backend::update`]). The endpoint and the public key that
//! release files must be signed with are in `tauri.conf.json`.

use std::sync::Mutex;
use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::backend::update::{CheckFuture, InstallFuture, Progress, Release, Updater};

/// How long a check may wait. Small: it runs at every launch.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the download of the installer (about 20 MiB) may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub struct TauriUpdater {
    app: AppHandle,
    /// What the last check found, for `install`.
    found: Mutex<Option<Update>>,
}

impl TauriUpdater {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            found: Mutex::new(None),
        }
    }
}

impl Updater for TauriUpdater {
    fn check(&self) -> CheckFuture<'_> {
        Box::pin(async move {
            let updater = self
                .app
                .updater_builder()
                .timeout(CHECK_TIMEOUT)
                .build()
                .map_err(|error| error.to_string())?;
            let update = updater.check().await.map_err(|error| error.to_string())?;
            let release = update.as_ref().map(|update| Release {
                version: update.version.clone(),
                notes: update.body.clone(),
                published_at: update.date.map(|date| date.unix_timestamp()),
            });
            *self.found.lock().expect("found update") = update;
            Ok(release)
        })
    }

    fn install(&self, version: String, mut progress: Progress) -> InstallFuture<'_> {
        Box::pin(async move {
            let found = self.found.lock().expect("found update").clone();
            let Some(mut update) = found.filter(|update| update.version == version) else {
                return Err(format!("version {version} was not found by the last check"));
            };
            update.timeout = Some(DOWNLOAD_TIMEOUT);
            let mut downloaded = 0u64;
            update
                .download_and_install(
                    |chunk, total| {
                        downloaded += chunk as u64;
                        progress(downloaded, total);
                    },
                    || {},
                )
                .await
                .map_err(|error| error.to_string())
        })
    }
}
