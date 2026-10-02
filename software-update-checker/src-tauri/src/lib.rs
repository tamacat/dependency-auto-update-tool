//! Software Update Checker のバックエンド。
//!
//! フロントエンド（WebView）は表示だけを担当し、ファイル読込・外部 API 呼び出し・
//! 判定ロジックはすべてこちらで行う。フロントから呼べるのは [`commands`] の関数だけ。
//! 外部への通信は [`http::Http`] に集約し、[`sources`] に登録した先にしか行わない。

// tr! マクロを後続のモジュールで使えるよう最初に読み込む
#[macro_use]
mod i18n;
mod activity;
mod checks;
mod commands;
mod http;
mod importers;
mod knowledge;
mod model;
mod netguard;
mod purl;
mod sources;
mod version;

use std::sync::Arc;
use tauri::{Emitter, Manager};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let log = Arc::new(activity::ActivityLog::new(
                app.path().app_log_dir().ok(),
                Some(Box::new(move |entry: &activity::LogEntry| {
                    let _ = handle.emit("activity-log", entry);
                })),
            ));
            log.info(tr!("起動: software-update-checker {}", "Started software-update-checker {}", env!("CARGO_PKG_VERSION")));
            let knowledge_path = app.path().app_config_dir().ok().map(|d| d.join("java-requirements.json"));
            let store = knowledge::KnowledgeStore::open(knowledge_path);
            if let Some(e) = store.load_error() {
                log.info(tr!(
                    "Java 要件ナレッジを読めません（修正するまで記録は保存しません）: {e}",
                    "Cannot read the Java requirements knowledge file (nothing will be saved until it is fixed): {e}"
                ));
            }
            app.manage(store);
            let cache_path = app.path().app_cache_dir().ok().map(|d| d.join("http-cache.json"));
            app.manage(http::Http::new(cache_path, log));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::import_file,
            commands::generate_sbom_with_maven,
            commands::run_checks,
            commands::java_requirements,
            commands::knowledge_list,
            commands::knowledge_upsert,
            commands::knowledge_delete,
            commands::knowledge_import,
            commands::knowledge_export,
            commands::data_sources,
            commands::set_language,
            commands::activity_log,
            commands::log_directory,
            commands::export_report,
            commands::clear_cache,
        ])
        .run(tauri::generate_context!())
        .expect("アプリケーションの起動に失敗しました");
}
