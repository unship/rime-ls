//! Watch config files for changes: Rime user_data_dir (auto-deploy) and rime-ls config.yaml (reload).

use crate::config;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// Start watching config paths. When Rime user_data_dir yaml changes, calls deploy().
/// When rime-ls config.yaml changes, calls reload_config_from_file().
///
/// Only call once after Rime is initialized. Runs in a background thread.
pub fn spawn_config_watcher(
    user_data_dir: impl AsRef<Path>,
    rime_ls_config_path: impl AsRef<Path>,
) {
    let user_data_dir = user_data_dir.as_ref().to_path_buf();
    let rime_ls_config_path = rime_ls_config_path.as_ref().to_path_buf();

    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();

        let mut debouncer = match new_debouncer(
            Duration::from_millis(500),
            move |res: DebounceEventResult| {
                let _ = tx.send(res);
            },
        ) {
            Ok(d) => d,
            Err(_) => return,
        };

        let watcher = debouncer.watcher();

        if user_data_dir.exists() {
            let _ = watcher.watch(&user_data_dir, notify::RecursiveMode::Recursive);
        }

        if let Some(parent) = rime_ls_config_path.parent() {
            let _ = watcher.watch(parent, notify::RecursiveMode::Recursive);
        }

        while let Ok(res) = rx.recv() {
            match res {
                Ok(events) => {
                    for e in &events {
                        let path = &e.path;
                        let is_yaml = path
                            .extension()
                            .map(|ext| ext == "yaml" || ext == "yml")
                            .unwrap_or(false);

                        if is_yaml {
                            if path.starts_with(&user_data_dir) && crate::rime::Rime::is_initialized()
                            {
                                crate::rime::Rime::global().deploy();
                                break;
                            }
                            if path.as_path() == rime_ls_config_path.as_path() {
                                config::reload_config_from_file();
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    crate::logger::error(format!("rime-ls config watcher error: {:?}", e));
                }
            }
        }
    });
}
