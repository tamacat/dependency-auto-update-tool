//! Software Update Checker の取り込みとチェックの処理。
//!
//! GUI（src-tauri）と CLI（cli）の両方から使う。Tauri には依存しないので、
//! WebKit や GTK の無い CI でもビルドできる。
//! 外部への通信は [`http::Http`] に集約し、[`sources`] に登録した先にしか行わない。

// tr! マクロを後続のモジュールで使えるよう最初に読み込む
#[macro_use]
pub mod i18n;
pub mod activity;
pub mod checks;
pub mod http;
pub mod importers;
pub mod knowledge;
pub mod model;
pub mod netguard;
pub mod policy;
pub mod purl;
pub mod sources;
pub mod version;
