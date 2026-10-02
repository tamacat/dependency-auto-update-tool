//! 外部サービスへ負荷をかけないための流量制御。
//!
//! [`crate::http::Http`] は送信のたびに [`Gate::admit`] で許可を取り、結果を [`Gate::report`] で返す。
//! - ホストごとの最小間隔と 1 分あたりの上限（超えそうなら待つ）
//! - 同じ要求（URL・Range・本文が同じ）を 1 分間に何度も送らない（画面側の不具合などによる
//!   暴走を止める。超えたら遮断）
//! - 連続して失敗したホストへは一定時間送らない（サーキットブレーカー）
//!
//! リトライ回数・待ち時間を含む設定値は [`NetworkSettings`]。利用者が設定画面で変えられるが、
//! 送信側に負荷をかけすぎない範囲に [`NetworkSettings::clamped`] で必ず丸める。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkSettings {
    /// 失敗時の再試行回数（0〜5）
    pub max_retries: u32,
    /// 再試行の待ち時間の基準（ミリ秒）。回数ごとに倍にする（200〜30000）
    pub retry_base_ms: u64,
    /// 1 回の待ち時間の上限（ミリ秒）。Retry-After がこれを超えたら再試行しない（1000〜120000）
    pub max_retry_wait_ms: u64,
    /// 同時に送る要求の数（1〜16）
    pub max_concurrency: usize,
    /// 同じホストへの要求の最小間隔（ミリ秒、0〜5000）
    pub min_interval_ms: u64,
    /// 同じホストへの 1 分あたりの要求数の上限（10〜1200）
    pub max_requests_per_minute: u32,
    /// 同じ要求を 1 分間に送ってよい回数（1〜20）。超えたら遮断する
    pub max_same_request_per_minute: u32,
    /// この回数連続で失敗したホストへの送信を一時停止する（1〜50）
    pub failure_threshold: u32,
    /// 一時停止する秒数（10〜3600）
    pub cooldown_secs: u64,
    /// 1 要求のタイムアウト（秒、5〜300）
    pub timeout_secs: u64,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            max_retries: 2,
            retry_base_ms: 1000,
            max_retry_wait_ms: 30_000,
            max_concurrency: 6,
            min_interval_ms: 100,
            max_requests_per_minute: 300,
            max_same_request_per_minute: 3,
            failure_threshold: 5,
            cooldown_secs: 120,
            timeout_secs: 60,
        }
    }
}

impl NetworkSettings {
    /// 安全な範囲に丸めた値。画面からどんな値が来ても、これ以上の負荷はかけない。
    pub fn clamped(&self) -> Self {
        Self {
            max_retries: self.max_retries.min(5),
            retry_base_ms: self.retry_base_ms.clamp(200, 30_000),
            max_retry_wait_ms: self.max_retry_wait_ms.clamp(1000, 120_000),
            max_concurrency: self.max_concurrency.clamp(1, 16),
            min_interval_ms: self.min_interval_ms.min(5000),
            max_requests_per_minute: self.max_requests_per_minute.clamp(10, 1200),
            max_same_request_per_minute: self.max_same_request_per_minute.clamp(1, 20),
            failure_threshold: self.failure_threshold.clamp(1, 50),
            cooldown_secs: self.cooldown_secs.clamp(10, 3600),
            timeout_secs: self.timeout_secs.clamp(5, 300),
        }
    }

    /// `attempt` 回目（1 始まり）の再試行までの待ち時間。`jitter` は 0.0〜1.0。
    pub fn backoff(&self, attempt: u32, jitter: f64) -> Duration {
        let exp = self.retry_base_ms.saturating_mul(1u64 << attempt.saturating_sub(1).min(10));
        let jittered = exp + (self.retry_base_ms as f64 * 0.25 * jitter) as u64;
        Duration::from_millis(jittered.min(self.max_retry_wait_ms))
    }
}

#[derive(Debug, PartialEq)]
pub enum Admission {
    Go,
    /// この時間待ってから再度 `admit` する
    Wait(Duration),
    /// 送ってはいけない（理由）
    Deny(String),
}

#[derive(Default)]
struct HostState {
    last: Option<Instant>,
    window: VecDeque<Instant>,
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

#[derive(Default)]
pub struct Gate {
    hosts: HashMap<String, HostState>,
    requests: HashMap<String, VecDeque<Instant>>,
}

fn prune(q: &mut VecDeque<Instant>, now: Instant) {
    while q.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
        q.pop_front();
    }
}

impl Gate {
    /// 送信してよいか。`request_key` は同じ要求の判定に使う（再試行のときは `None`）。
    pub fn admit(&mut self, host: &str, request_key: Option<&str>, now: Instant, s: &NetworkSettings) -> Admission {
        let state = self.hosts.entry(host.to_string()).or_default();
        if let Some(until) = state.open_until {
            if now < until {
                return Admission::Deny(tr!(
                    "{host} への送信を一時停止中です（{} 回連続で失敗、あと {} 秒）",
                    "Requests to {host} are paused ({} consecutive failures, {} s left)",
                    state.consecutive_failures,
                    (until - now).as_secs() + 1
                ));
            }
            state.open_until = None;
        }

        if let Some(key) = request_key {
            let q = self.requests.entry(key.to_string()).or_default();
            prune(q, now);
            if q.len() as u32 >= s.max_same_request_per_minute {
                return Admission::Deny(tr!(
                    "同じ要求が 1 分間に {} 回を超えたため遮断しました（繰り返し要求の防止）",
                    "Blocked: the same request was sent more than {} times in a minute (repeated request protection)",
                    s.max_same_request_per_minute
                ));
            }
        }

        prune(&mut state.window, now);
        if state.window.len() as u32 >= s.max_requests_per_minute {
            let oldest = *state.window.front().expect("上限 > 0");
            return Admission::Wait(WINDOW - now.duration_since(oldest));
        }
        if let Some(last) = state.last {
            let interval = Duration::from_millis(s.min_interval_ms);
            let since = now.duration_since(last);
            if since < interval {
                return Admission::Wait(interval - since);
            }
        }

        state.last = Some(now);
        state.window.push_back(now);
        if let Some(key) = request_key {
            self.requests.entry(key.to_string()).or_default().push_back(now);
        }
        Admission::Go
    }

    /// 送信結果を記録する。`ok` は相手が正常に応答したか（404 も正常、5xx・429・通信失敗は異常）。
    /// 一時停止に入ったら `true`。
    pub fn report(&mut self, host: &str, ok: bool, now: Instant, s: &NetworkSettings) -> bool {
        let state = self.hosts.entry(host.to_string()).or_default();
        if ok {
            state.consecutive_failures = 0;
            return false;
        }
        state.consecutive_failures += 1;
        if state.consecutive_failures >= s.failure_threshold && state.open_until.is_none() {
            state.open_until = Some(now + Duration::from_secs(s.cooldown_secs));
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> NetworkSettings {
        NetworkSettings { min_interval_ms: 100, max_requests_per_minute: 10, max_same_request_per_minute: 2, ..Default::default() }
    }

    #[test]
    fn enforces_min_interval_per_host() {
        let s = settings();
        let mut g = Gate::default();
        let t = Instant::now();
        assert_eq!(g.admit("a", Some("1"), t, &s), Admission::Go);
        assert_eq!(g.admit("a", Some("2"), t + Duration::from_millis(30), &s), Admission::Wait(Duration::from_millis(70)));
        // 別ホストは独立
        assert_eq!(g.admit("b", Some("3"), t + Duration::from_millis(30), &s), Admission::Go);
        assert_eq!(g.admit("a", Some("2"), t + Duration::from_millis(100), &s), Admission::Go);
    }

    #[test]
    fn waits_when_per_minute_limit_reached() {
        let s = NetworkSettings { min_interval_ms: 0, ..settings() };
        let mut g = Gate::default();
        let t = Instant::now();
        for i in 0..10 {
            assert_eq!(g.admit("a", Some(&i.to_string()), t + Duration::from_secs(i), &s), Admission::Go);
        }
        assert_eq!(g.admit("a", Some("x"), t + Duration::from_secs(10), &s), Admission::Wait(Duration::from_secs(50)));
        assert_eq!(g.admit("a", Some("x"), t + Duration::from_secs(60), &s), Admission::Go);
    }

    #[test]
    fn denies_repeated_identical_requests_but_not_retries() {
        let s = NetworkSettings { min_interval_ms: 0, ..settings() };
        let mut g = Gate::default();
        let t = Instant::now();
        assert_eq!(g.admit("a", Some("same"), t, &s), Admission::Go);
        assert_eq!(g.admit("a", None, t, &s), Admission::Go); // 再試行は数えない
        assert_eq!(g.admit("a", Some("same"), t, &s), Admission::Go);
        assert!(matches!(g.admit("a", Some("same"), t, &s), Admission::Deny(_)));
        // 1 分たてば再び送れる
        assert_eq!(g.admit("a", Some("same"), t + Duration::from_secs(61), &s), Admission::Go);
    }

    #[test]
    fn deny_reason_follows_display_language() {
        use crate::i18n::{with_language, Language};
        let s = NetworkSettings { min_interval_ms: 0, max_same_request_per_minute: 1, ..settings() };
        let mut g = Gate::default();
        let t = Instant::now();
        assert_eq!(g.admit("a", Some("k"), t, &s), Admission::Go);
        let en = with_language(Language::English, || g.admit("a", Some("k"), t, &s));
        assert!(matches!(&en, Admission::Deny(m) if m.starts_with("Blocked: the same request")), "{en:?}");
        let ja = with_language(Language::Japanese, || g.admit("a", Some("k"), t, &s));
        assert!(matches!(&ja, Admission::Deny(m) if m.starts_with("同じ要求")), "{ja:?}");
    }

    #[test]
    fn opens_circuit_after_consecutive_failures() {
        let s = NetworkSettings { failure_threshold: 3, cooldown_secs: 30, min_interval_ms: 0, ..settings() };
        let mut g = Gate::default();
        let t = Instant::now();
        assert!(!g.report("a", false, t, &s));
        assert!(!g.report("a", true, t, &s)); // 成功で数え直し
        assert!(!g.report("a", false, t, &s));
        assert!(!g.report("a", false, t, &s));
        assert!(g.report("a", false, t, &s));
        assert!(matches!(g.admit("a", Some("k"), t + Duration::from_secs(10), &s), Admission::Deny(_)));
        assert_eq!(g.admit("a", Some("k"), t + Duration::from_secs(31), &s), Admission::Go);
    }

    #[test]
    fn clamps_unsafe_settings() {
        let s = NetworkSettings {
            max_retries: 100,
            retry_base_ms: 1,
            max_concurrency: 1000,
            max_requests_per_minute: 1_000_000,
            ..Default::default()
        }
        .clamped();
        assert_eq!(s.max_retries, 5);
        assert_eq!(s.retry_base_ms, 200);
        assert_eq!(s.max_concurrency, 16);
        assert_eq!(s.max_requests_per_minute, 1200);
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let s = NetworkSettings { retry_base_ms: 1000, max_retry_wait_ms: 3000, ..Default::default() };
        assert_eq!(s.backoff(1, 0.0), Duration::from_millis(1000));
        assert_eq!(s.backoff(2, 0.0), Duration::from_millis(2000));
        assert_eq!(s.backoff(2, 1.0), Duration::from_millis(2250));
        assert_eq!(s.backoff(5, 0.0), Duration::from_millis(3000));
    }
}
