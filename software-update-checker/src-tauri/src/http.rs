//! 外部 API 呼び出し、その応答のファイルキャッシュ、通信の許可制御・流量制御と記録。
//!
//! - 接続先は [`crate::sources`] に登録されたホストのうち、利用者が有効にしている
//!   データソースのものだけ。それ以外は送信前に遮断する。
//! - 送信はすべて [`crate::netguard::Gate`] を通す（ホストごとの間隔・上限、同じ要求の
//!   繰り返し防止、連続失敗時の一時停止）。再試行の回数と待ち時間も設定値に従う。
//! - すべての要求（キャッシュ利用・遮断・失敗・再試行を含む）を [`ActivityLog`] に記録する。
//! - キャッシュは「キー → ステータスと本文」の単純な JSON ファイル。依存を増やさないため
//!   SQLite は使っていない。件数が増えて重くなったらここだけ差し替えればよい。

use crate::activity::{ActivityLog, LogEntry};
use crate::model::CheckOptions;
use crate::netguard::{Admission, Gate, NetworkSettings};
use crate::sources;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// 流量制御で待つ時間の合計の上限。これを超えるなら送信をあきらめる。
const MAX_GATE_WAIT: Duration = Duration::from_secs(120);
/// 応答本文の大きさの上限。最大の endoflife.date の製品一覧でも 3MB 程度。
const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    stored_at: u64,
    status: u16,
    body: String,
}

#[derive(Default)]
struct CacheData {
    entries: HashMap<String, Entry>,
    dirty: bool,
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    cache: Arc<Mutex<CacheData>>,
    cache_path: Option<PathBuf>,
    disabled_sources: Arc<RwLock<HashSet<String>>>,
    network: Arc<RwLock<NetworkSettings>>,
    gate: Arc<std::sync::Mutex<Gate>>,
    log: Arc<ActivityLog>,
}

/// 応答。404 などは `Ok` で返し、通信失敗と遮断だけを `Err` にする。
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// 部分取得（Range）の応答。
pub struct BytesResponse {
    pub status: u16,
    pub body: Vec<u8>,
    /// `Content-Range` の全体サイズ
    pub total_size: Option<u64>,
}

struct RawResponse {
    status: u16,
    body: Vec<u8>,
    total_size: Option<u64>,
    retry_after: Option<Duration>,
}

/// 1 回の送信の内容。
struct Request<'a> {
    source: &'static str,
    host: String,
    method: &'static str,
    url: &'a str,
    /// 同じ要求の判定に使うキー（URL・Range・本文から作る）
    key: String,
    /// ログに残す送信内容の要約
    summary: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 再試行の待ち時間をずらすための 0.0〜1.0 の値（乱数用の依存を増やさないため時刻から作る）。
fn jitter() -> f64 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    f64::from(nanos % 1000) / 1000.0
}

/// ログに残す URL。長すぎるものは切り詰める。
fn url_for_log(url: &str) -> String {
    match url.char_indices().nth(500) {
        Some((i, _)) => format!("{}…", &url[..i]),
        None => url.to_string(),
    }
}

fn hash_of(value: &str) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

impl Http {
    pub fn new(cache_path: Option<PathBuf>, log: Arc<ActivityLog>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("software-update-checker/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP クライアントを初期化できません");

        let entries = cache_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<HashMap<String, Entry>>(&s).ok())
            .unwrap_or_default();

        Self {
            client,
            cache: Arc::new(Mutex::new(CacheData { entries, dirty: false })),
            cache_path,
            disabled_sources: Arc::new(RwLock::new(HashSet::new())),
            network: Arc::new(RwLock::new(NetworkSettings::default())),
            gate: Arc::new(std::sync::Mutex::new(Gate::default())),
            log,
        }
    }

    pub fn log(&self) -> &ActivityLog {
        &self.log
    }

    /// 設定画面の値（無効なデータソース・通信設定）を反映する。処理を始める前に必ず呼ぶ。
    pub fn configure(&self, options: &CheckOptions) {
        if let Ok(mut d) = self.disabled_sources.write() {
            *d = options.disabled_sources.iter().cloned().collect();
        }
        if let Ok(mut n) = self.network.write() {
            *n = options.network.clamped();
        }
    }

    pub fn network(&self) -> NetworkSettings {
        self.network.read().map(|n| n.clone()).unwrap_or_default()
    }

    pub fn is_source_enabled(&self, id: &str) -> bool {
        self.disabled_sources.read().map(|d| !d.contains(id)).unwrap_or(false)
    }

    fn blocked(&self, source: Option<&str>, method: &str, url: &str, reason: &str) -> String {
        self.log.record(LogEntry {
            kind: "http".into(),
            source: source.map(str::to_string),
            method: Some(method.into()),
            url: Some(url_for_log(url)),
            outcome: "blocked".into(),
            message: Some(reason.to_string()),
            ..Default::default()
        });
        tr!("通信を遮断しました: {reason}", "Request blocked: {reason}")
    }

    /// 接続してよい URL か。よければ (データソース ID, ホスト) を返す。
    fn authorize(&self, method: &str, url: &str) -> Result<(&'static str, String), String> {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let reason = match sources::by_host(&host) {
            Some(s) if self.is_source_enabled(s.id) => return Ok((s.id, host)),
            Some(s) => tr!("データソース「{}」は設定で無効になっています", "The data source \"{}\" is disabled in the settings", s.localized_name()),
            None => tr!("{host} は登録されていない接続先です", "{host} is not a registered destination"),
        };
        Err(self.blocked(sources::by_host(&host).map(|s| s.id), method, url, &reason))
    }

    fn record_cache_hit(&self, source: &str, method: &str, url: &str, response: &Response) {
        self.log.record(LogEntry {
            kind: "http".into(),
            source: Some(source.into()),
            method: Some(method.into()),
            url: Some(url_for_log(url)),
            status: Some(response.status),
            bytes: Some(response.body.len()),
            outcome: "cache".into(),
            ..Default::default()
        });
    }

    async fn cached(&self, key: &str, ttl: Duration) -> Option<Response> {
        let cache = self.cache.lock().await;
        let entry = cache.entries.get(key)?;
        if now_secs().saturating_sub(entry.stored_at) > ttl.as_secs() {
            return None;
        }
        Some(Response { status: entry.status, body: entry.body.clone() })
    }

    async fn store(&self, key: String, response: &Response) {
        // 5xx や 429 は一時的な失敗なのでキャッシュしない
        if response.status >= 500 || response.status == 429 {
            return;
        }
        let mut cache = self.cache.lock().await;
        cache.entries.insert(
            key,
            Entry { stored_at: now_secs(), status: response.status, body: response.body.clone() },
        );
        cache.dirty = true;
    }

    /// GET。`ttl` が `None` ならキャッシュを使わない（結果は保存する）。
    pub async fn get(&self, url: &str, ttl: Option<Duration>) -> Result<Response, String> {
        let (source, host) = self.authorize("GET", url)?;
        if let Some(ttl) = ttl {
            if let Some(hit) = self.cached(url, ttl).await {
                self.record_cache_hit(source, "GET", url, &hit);
                return Ok(hit);
            }
        }
        let req = Request { source, host, method: "GET", url, key: format!("GET {url}"), summary: None };
        let raw = self.execute(&req, || self.client.get(url)).await?;
        let response = Response { status: raw.status, body: String::from_utf8_lossy(&raw.body).into_owned() };
        self.store(url.to_string(), &response).await;
        Ok(response)
    }

    /// JSON 本文付き POST（OSV の querybatch 用）。キャッシュのキーは URL と本文の組。
    /// `summary` は送信内容の要約としてログに残す。
    pub async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        ttl: Option<Duration>,
        summary: &str,
    ) -> Result<Response, String> {
        let (source, host) = self.authorize("POST", url)?;
        let body_text = body.to_string();
        let cache_key = format!("POST {url} {body_text}");
        if let Some(ttl) = ttl {
            if let Some(hit) = self.cached(&cache_key, ttl).await {
                self.record_cache_hit(source, "POST", url, &hit);
                return Ok(hit);
            }
        }
        let req = Request {
            source,
            host,
            method: "POST",
            url,
            key: format!("POST {url} #{:x}", hash_of(&body_text)),
            summary: Some(summary.to_string()),
        };
        let raw = self.execute(&req, || self.client.post(url).json(body)).await?;
        let response = Response { status: raw.status, body: String::from_utf8_lossy(&raw.body).into_owned() };
        self.store(cache_key, &response).await;
        Ok(response)
    }

    /// Range 指定の部分取得（キャッシュしない）。`range` は `bytes=` 以降の値。
    pub async fn get_range(&self, url: &str, range: &str) -> Result<BytesResponse, String> {
        let (source, host) = self.authorize("GET", url)?;
        let req = Request {
            source,
            host,
            method: "GET",
            url,
            key: format!("GET {url} bytes={range}"),
            summary: Some(tr!("部分取得 bytes={range}", "Partial download bytes={range}")),
        };
        let raw = self
            .execute(&req, || self.client.get(url).header(reqwest::header::RANGE, format!("bytes={range}")))
            .await?;
        Ok(BytesResponse { status: raw.status, body: raw.body, total_size: raw.total_size })
    }

    /// 流量制御の許可を待つ。遮断なら理由を返す。
    async fn admit(&self, host: &str, key: Option<&str>, s: &NetworkSettings) -> Result<Duration, String> {
        let mut waited = Duration::ZERO;
        loop {
            let admission = match self.gate.lock() {
                Ok(mut g) => g.admit(host, key, Instant::now(), s),
                Err(_) => return Err(tr!("流量制御の状態を読めません", "Cannot read the rate limiter state")),
            };
            match admission {
                Admission::Go => return Ok(waited),
                Admission::Deny(reason) => return Err(reason),
                Admission::Wait(d) => {
                    if waited + d > MAX_GATE_WAIT {
                        return Err(tr!(
                            "{host} への送信上限（1 分あたり {} 件）のため待ち時間が長くなりすぎました",
                            "Waited too long because of the request limit for {host} ({} per minute)",
                            s.max_requests_per_minute
                        ));
                    }
                    tokio::time::sleep(d).await;
                    waited += d;
                }
            }
        }
    }

    /// 送信本体。流量制御・再試行・記録をまとめて行う。
    async fn execute(
        &self,
        req: &Request<'_>,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<RawResponse, String> {
        let s = self.network();
        let mut last_error = String::new();

        for attempt in 0..=s.max_retries {
            let is_retry = attempt > 0;
            let waited = match self.admit(&req.host, (!is_retry).then_some(req.key.as_str()), &s).await {
                Ok(w) => w,
                Err(reason) => return Err(self.blocked(Some(req.source), req.method, req.url, &reason)),
            };

            let started = Instant::now();
            let outcome = async {
                let mut res = build().timeout(Duration::from_secs(s.timeout_secs)).send().await.map_err(|e| e.to_string())?;
                let status = res.status().as_u16();
                let header = |name| res.headers().get(name).and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok());
                let total_size = header(reqwest::header::CONTENT_RANGE).and_then(|v| v.rsplit('/').next()?.parse().ok());
                let retry_after = header(reqwest::header::RETRY_AFTER).and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs);
                let too_large = || {
                    tr!(
                        "応答が大きすぎるため打ち切りました（上限 {} MB）",
                        "Response too large; aborted (limit {} MB)",
                        MAX_BODY_BYTES / 1024 / 1024
                    )
                };
                if res.content_length().is_some_and(|n| n > MAX_BODY_BYTES as u64) {
                    return Err(too_large());
                }
                let mut body = Vec::new();
                while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
                    if body.len() + chunk.len() > MAX_BODY_BYTES {
                        return Err(too_large());
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok::<_, String>(RawResponse { status, body, total_size, retry_after })
            }
            .await;

            let healthy = matches!(&outcome, Ok(r) if r.status != 429 && r.status < 500);
            let opened = self.gate.lock().map(|mut g| g.report(&req.host, healthy, Instant::now(), &s)).unwrap_or(false);

            let mut notes: Vec<String> = Vec::new();
            if let Some(summary) = &req.summary {
                notes.push(summary.clone());
            }
            if is_retry {
                notes.push(tr!("再試行 {attempt}/{}", "Retry {attempt}/{}", s.max_retries));
            }
            if waited >= Duration::from_millis(500) {
                notes.push(tr!("流量制御で {} ms 待機", "Waited {} ms for rate limiting", waited.as_millis()));
            }
            if let Err(e) = &outcome {
                notes.push(e.clone());
            }
            if opened {
                notes.push(tr!(
                    "{} 回連続で失敗したため、{} への送信を {} 秒停止します",
                    "{} consecutive failures; pausing requests to {} for {} s",
                    s.failure_threshold, req.host, s.cooldown_secs
                ));
            }
            self.log.record(LogEntry {
                kind: "http".into(),
                source: Some(req.source.into()),
                method: Some(req.method.into()),
                url: Some(url_for_log(req.url)),
                status: outcome.as_ref().ok().map(|r| r.status),
                duration_ms: Some(started.elapsed().as_millis() as u64),
                bytes: outcome.as_ref().ok().map(|r| r.body.len()),
                outcome: if healthy { "network" } else { "error" }.into(),
                message: (!notes.is_empty()).then(|| notes.join(" / ")),
                ..Default::default()
            });

            let retry_after = match outcome {
                Ok(r) if healthy => return Ok(r),
                Ok(r) => {
                    last_error = format!("HTTP {}", r.status);
                    // 4xx（429 以外）は再試行しても結果が変わらない
                    if r.status != 429 && r.status < 500 {
                        return Ok(r);
                    }
                    r.retry_after
                }
                Err(e) => {
                    last_error = e;
                    None
                }
            };
            if attempt == s.max_retries {
                break;
            }
            let wait = match retry_after {
                Some(d) if d.as_millis() as u64 > s.max_retry_wait_ms => {
                    last_error = tr!(
                        "{last_error}（Retry-After {} 秒が待ち時間の上限を超えるため再試行しません）",
                        "{last_error} (not retrying: Retry-After {} s exceeds the maximum wait)",
                        d.as_secs()
                    );
                    break;
                }
                Some(d) => d,
                None => s.backoff(attempt + 1, jitter()),
            };
            tokio::time::sleep(wait).await;
        }
        Err(last_error)
    }

    /// 変更があればキャッシュファイルへ書き出す。
    pub async fn save(&self) -> Result<(), String> {
        let Some(path) = &self.cache_path else { return Ok(()) };
        let mut cache = self.cache.lock().await;
        if !cache.dirty {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string(&cache.entries).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())?;
        cache.dirty = false;
        Ok(())
    }

    pub async fn clear(&self) -> Result<(), String> {
        let mut cache = self.cache.lock().await;
        cache.entries.clear();
        cache.dirty = false;
        if let Some(path) = &self.cache_path {
            if path.exists() {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_with(options: CheckOptions) -> (Http, Arc<ActivityLog>) {
        let log = Arc::new(ActivityLog::disabled());
        let http = Http::new(None, log.clone());
        http.configure(&options);
        (http, log)
    }

    #[tokio::test]
    async fn blocks_disabled_and_unknown_hosts_without_sending() {
        let (http, log) = http_with(CheckOptions { disabled_sources: vec![sources::OSV.into()], ..Default::default() });

        let err = http.get("https://api.osv.dev/v1/vulns/X", None).await.err().unwrap();
        assert!(err.contains("遮断"));
        let err = http.get("https://example.com/", None).await.err().unwrap();
        assert!(err.contains("登録されていない"));

        let entries = log.recent();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.outcome == "blocked"));
        assert_eq!(entries[0].source.as_deref(), Some(sources::OSV));
    }

    #[tokio::test]
    async fn logs_cache_hits_as_no_network() {
        let (http, log) = http_with(CheckOptions::default());
        let url = "https://api.deps.dev/v3/systems/MAVEN/packages/x";
        http.store(url.to_string(), &Response { status: 200, body: "{}".into() }).await;
        let res = http.get(url, Some(Duration::from_secs(60))).await.unwrap();
        assert_eq!(res.body, "{}");
        assert_eq!(log.recent()[0].outcome, "cache");
    }

    /// 決まった応答を返し続ける手元の HTTP サーバー。受けた要求の数を返す。
    fn serve(response: &'static str) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (url, hits)
    }

    fn fast_retry_options() -> CheckOptions {
        CheckOptions {
            network: NetworkSettings {
                max_retries: 2,
                retry_base_ms: 200,
                min_interval_ms: 0,
                failure_threshold: 4,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn retries_up_to_limit_then_pauses_the_host() {
        let (url, hits) =
            serve("HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let (http, log) = http_with(fast_retry_options());

        // 1 回目の要求: 初回 + 再試行 2 回 = 3 回送って失敗
        let err = http.get(&format!("{url}a"), None).await.err().unwrap();
        assert_eq!(err, "HTTP 503");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);

        // 2 回目の要求: 4 回目の失敗で一時停止に入り、それ以上は送らない
        let err = http.get(&format!("{url}b"), None).await.err().unwrap();
        assert!(err.contains("一時停止"), "{err}");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 4);

        let entries = log.recent();
        assert!(entries.iter().any(|e| e.message.as_deref().is_some_and(|m| m.contains("秒停止します"))));
        assert_eq!(entries.last().unwrap().outcome, "blocked");
    }

    #[tokio::test]
    async fn does_not_retry_when_retry_after_exceeds_limit() {
        let (url, hits) =
            serve("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 3600\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let (http, _) = http_with(fast_retry_options());
        let err = http.get(&url, None).await.err().unwrap();
        assert!(err.contains("Retry-After 3600"), "{err}");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rejects_oversized_responses() {
        let (url, _) =
            serve("HTTP/1.1 200 OK\r\nContent-Length: 104857600\r\nConnection: close\r\n\r\npartial-body");
        let (http, _) = http_with(CheckOptions {
            network: NetworkSettings { max_retries: 0, ..fast_retry_options().network },
            ..Default::default()
        });
        let err = http.get(&url, None).await.err().unwrap();
        assert!(err.contains("大きすぎる"), "{err}");
    }

    #[tokio::test]
    async fn does_not_retry_client_errors() {
        let (url, hits) = serve("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let (http, _) = http_with(fast_retry_options());
        let res = http.get(&url, None).await.unwrap();
        assert_eq!(res.status, 404);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn blocks_runaway_identical_requests_before_sending() {
        let (http, log) = http_with(CheckOptions::default());
        let s = http.network();
        let url = "https://repo1.maven.org/maven2/x.jar";
        let key = format!("GET {url} bytes=-65536");
        // 上限まで送信済みの状態を作る（実際には送らない）
        {
            let mut g = http.gate.lock().unwrap();
            let t = Instant::now() - Duration::from_secs(5);
            for i in 0..s.max_same_request_per_minute {
                assert_eq!(g.admit("repo1.maven.org", Some(&key), t + Duration::from_secs(i as u64), &s), Admission::Go);
            }
        }
        let err = http.get_range(url, "-65536").await.err().unwrap();
        assert!(err.contains("繰り返し"));
        assert_eq!(log.recent().last().unwrap().outcome, "blocked");
    }
}
