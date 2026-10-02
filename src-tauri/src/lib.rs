//! Classeur URSSAF : coque Tauri. La logique (chiffrement, stockage, Git) est dans `classeur-core`.

mod app;
mod commands;
mod config;
mod remote_url;
mod secrets;

#[cfg(test)]
mod e2e_tests;

use tauri::Manager;

use crate::app::{spawn_background_worker, wipe_open_dir, AppState, Paths};
use crate::config::Config;

/// Liste unique des commandes exposées, partagée avec les tests d'intégration.
macro_rules! app_handlers {
    () => {
        tauri::generate_handler![
            $crate::commands::get_status,
            $crate::commands::touch,
            $crate::commands::setup_remote,
            $crate::commands::create_vault,
            $crate::commands::unlock,
            $crate::commands::lock,
            $crate::commands::list_records,
            $crate::commands::save_month,
            $crate::commands::delete_month,
            $crate::commands::add_attachments,
            $crate::commands::pick_attachments,
            $crate::commands::remove_attachment,
            $crate::commands::attachment_bytes,
            $crate::commands::open_attachment,
            $crate::commands::save_attachment_as,
            $crate::commands::sync_now,
            $crate::commands::set_token,
            $crate::commands::set_auto_lock,
            $crate::commands::change_passphrase,
            $crate::commands::reset_local,
        ]
    };
}
#[cfg(test)]
pub(crate) use app_handlers;

pub fn run() {
    classeur_core::sync::init_network_timeouts();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
        .setup(|app| {
            let resolver = app.path();
            let paths = Paths {
                data_dir: resolver.app_local_data_dir()?.join("data"),
                config_file: resolver.app_config_dir()?.join("config.json"),
                open_dir: resolver.app_cache_dir()?.join("open"),
            };
            // Des copies déchiffrées ont pu rester après un arrêt brutal : on les efface.
            wipe_open_dir(&paths.open_dir);

            let config = Config::load(&paths.config_file);
            let repo = if config.remote_url.is_some() {
                classeur_core::sync::DataRepo::open(&paths.data_dir).ok()
            } else {
                None
            };
            app.manage(AppState::new(paths, config, repo));

            // Mise à jour des données à chaque lancement (sans phrase secrète : fast-forward seulement).
            app::request_sync(app.handle());
            spawn_background_worker(app.handle().clone());
            Ok(())
        })
        .invoke_handler(app_handlers!())
        .build(tauri::generate_context!())
        .expect("erreur au démarrage de Classeur URSSAF")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = handle.try_state::<AppState>() {
                    wipe_open_dir(&state.paths.open_dir);
                }
            }
        });
}
