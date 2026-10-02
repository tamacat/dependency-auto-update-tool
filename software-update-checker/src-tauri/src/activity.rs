//! 通信・操作ログ。
//!
//! 外部への HTTP 通信（キャッシュ利用・遮断を含む）、Maven の実行、取り込みや
//! チェックの開始・終了を記録する。ログはアプリのログディレクトリに日別の
//! JSON Lines（`activity-YYYY-MM-DD.jsonl`、日付は UTC）で追記し、
//! [`RETENTION_DAYS`] 日より古いファイルは起動時に削除する。
//! 画面には直近の分をメモリから返し、新しい記録はイベントでも通知する。

use serde::Serialize;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const RETENTION_DAYS: u64 = 30;
const RECENT_CAPACITY: usize = 5000;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// UTC の ISO 8601（ミリ秒まで）
    pub ts: String,
    /// `http` / `process` / `app`
    pub kind: String,
    /// データソース ID（[`crate::sources`]）
    pub source: Option<String>,
    pub method: Option<String>,
    pub url: Option<String>,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub bytes: Option<usize>,
    /// `network`（実際に通信）/ `cache`（通信なし）/ `blocked`（設定で遮断）/ `error` / `info`
    pub outcome: String,
    pub message: Option<String>,
}

type Sink = Box<dyn Fn(&LogEntry) + Send + Sync>;

pub struct ActivityLog {
    dir: Option<PathBuf>,
    recent: Mutex<VecDeque<LogEntry>>,
    sink: Option<Sink>,
    /// ファイルへの追記を 1 件ずつにする（並行して書くと行が混ざるため）
    write_lock: Mutex<()>,
}

impl ActivityLog {
    pub fn new(dir: Option<PathBuf>, sink: Option<Sink>) -> Self {
        let log = Self { dir, recent: Mutex::new(VecDeque::new()), sink, write_lock: Mutex::new(()) };
        log.prune();
        log
    }

    /// ファイルにもイベントにも出さない（テスト用）。
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self { dir: None, recent: Mutex::new(VecDeque::new()), sink: None, write_lock: Mutex::new(()) }
    }

    pub fn dir(&self) -> Option<&PathBuf> {
        self.dir.as_ref()
    }

    pub fn record(&self, mut entry: LogEntry) {
        entry.ts = iso_utc(now_millis());

        if let Some(dir) = &self.dir {
            let file = dir.join(format!("activity-{}.jsonl", &entry.ts[..10]));
            // 1 行を 1 回の書き込みで出し、書き込み中は他の記録を待たせる
            let line = format!("{}\n", serde_json::to_string(&entry).unwrap_or_default());
            let _guard = self.write_lock.lock();
            let written = std::fs::create_dir_all(dir).and_then(|_| {
                let mut f = OpenOptions::new().create(true).append(true).open(&file)?;
                f.write_all(line.as_bytes())
            });
            if let Err(e) = written {
                eprintln!("通信ログを書き込めません（{}）: {e}", file.display());
            }
        }
        if let Some(sink) = &self.sink {
            sink(&entry);
        }
        if let Ok(mut recent) = self.recent.lock() {
            if recent.len() >= RECENT_CAPACITY {
                recent.pop_front();
            }
            recent.push_back(entry);
        }
    }

    pub fn info(&self, message: impl Into<String>) {
        self.record(LogEntry { kind: "app".into(), outcome: "info".into(), message: Some(message.into()), ..Default::default() });
    }

    pub fn recent(&self) -> Vec<LogEntry> {
        self.recent.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
    }

    /// 保持期間を過ぎたログファイルを削除する。
    fn prune(&self) {
        let Some(dir) = &self.dir else { return };
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let today = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 / 86_400;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(date) = name.strip_prefix("activity-").and_then(|n| n.strip_suffix(".jsonl")) else {
                continue;
            };
            if let Some(day) = crate::version::days_from_iso_date(date) {
                if today - day > RETENTION_DAYS as i64 {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

pub fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// エポックミリ秒を `YYYY-MM-DDTHH:MM:SS.mmmZ` にする。
pub fn iso_utc(epoch_millis: i64) -> String {
    let secs = epoch_millis.div_euclid(1000);
    let millis = epoch_millis.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Howard Hinnant の civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_710_854_871_123), "2024-03-19T13:27:51.123Z");
        assert_eq!(iso_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn concurrent_records_do_not_interleave() {
        let dir = std::env::temp_dir().join(format!("suc-log-concurrent-{}", std::process::id()));
        let log = std::sync::Arc::new(ActivityLog::new(Some(dir.clone()), None));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let log = log.clone();
                std::thread::spawn(move || {
                    for i in 0..200 {
                        log.record(LogEntry {
                            kind: "http".into(),
                            outcome: "network".into(),
                            url: Some(format!("https://example/{t}/{i}/{}", "x".repeat(300))),
                            ..Default::default()
                        });
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let mut lines = 0;
        for f in std::fs::read_dir(&dir).unwrap() {
            for line in std::fs::read_to_string(f.unwrap().path()).unwrap().lines() {
                serde_json::from_str::<serde_json::Value>(line).expect("1 行 1 件の JSON であること");
                lines += 1;
            }
        }
        assert_eq!(lines, 1600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_jsonl_and_keeps_recent() {
        let dir = std::env::temp_dir().join(format!("suc-log-test-{}", std::process::id()));
        let log = ActivityLog::new(Some(dir.clone()), None);
        log.info("hello");
        log.record(LogEntry { kind: "http".into(), outcome: "blocked".into(), url: Some("https://x".into()), ..Default::default() });
        assert_eq!(log.recent().len(), 2);
        let file = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\"outcome\":\"blocked\""));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
