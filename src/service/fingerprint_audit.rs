use chrono::{DateTime, Utc};
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

use crate::model::account::{Account, CanonicalEnvData};

const AUDIT_SCHEMA_VERSION: u32 = 1;
const AUDIT_FILE_NAME: &str = "fingerprint-audit.jsonl";
const AUDIT_MAX_SIZE_BYTES: u64 = 10 * 1024 * 1024;
const AUDIT_MAX_HISTORY_FILES: usize = 6;
const AUDIT_QUEUE_CAPACITY: usize = 1024;
const AUDIT_FLUSH_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AuditProfile {
    pub claude_version: String,
    pub build_time: String,
    pub stainless_package: String,
    pub platform: String,
    pub arch: String,
    pub stainless_os: String,
    pub runtime: String,
    pub runtime_version: String,
    pub is_running_with_bun: bool,
}

impl AuditProfile {
    pub fn from_env(env: &CanonicalEnvData) -> Self {
        let platform = match env.platform.as_str() {
            "darwin" => "darwin",
            "linux" => "linux",
            "win32" => "win32",
            _ => "other",
        };
        let arch = match env.arch.as_str() {
            "arm64" => "arm64",
            "x64" => "x64",
            "arm" => "arm",
            "x32" => "x32",
            _ => "other",
        };
        let stainless_os = match platform {
            "darwin" => "MacOS",
            "win32" => "Windows",
            "linux" => "Linux",
            _ => "Unknown",
        };
        Self {
            claude_version: crate::model::identity::CLAUDE_CODE_VERSION.into(),
            build_time: crate::model::identity::CLAUDE_CODE_BUILD_TIME.into(),
            stainless_package: crate::model::identity::CLAUDE_CODE_STAINLESS_VERSION.into(),
            platform: platform.into(),
            arch: arch.into(),
            stainless_os: stainless_os.into(),
            runtime: "node".into(),
            runtime_version: env.node_version.clone(),
            is_running_with_bun: env.is_running_with_bun,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FingerprintMismatch {
    Version,
    BuildTime,
    StainlessPackage,
    StainlessOs,
    RuntimeVersion,
    UaEntrypoint,
    Beta,
    Thinking,
    MissingSessionId,
    MissingClientRequestId,
}

#[derive(Debug, Clone)]
pub struct AuditRequestObservation {
    account_id: i64,
    auth_type: String,
    auto_telemetry: bool,
    profile: AuditProfile,
    client_type: String,
    entrypoint: String,
    model: String,
    stream: bool,
    thinking: Option<String>,
    effort: Option<String>,
    betas: Vec<String>,
    session_id_present: bool,
    client_request_id_present: bool,
    mismatches: Vec<FingerprintMismatch>,
}

#[derive(Debug, Clone)]
enum AuditMessage {
    Request(AuditRequestObservation),
    Response {
        account_id: i64,
        status: u16,
    },
    Telemetry {
        account_id: i64,
        event_name: &'static str,
        success: bool,
    },
    Session {
        account_id: i64,
        state: &'static str,
        profile: AuditProfile,
    },
}

#[derive(Debug, Default)]
struct AccountCounters {
    auth_type: String,
    auto_telemetry: bool,
    profile: Option<AuditProfile>,
    requests: u64,
    stream_requests: u64,
    status_2xx: u64,
    status_403: u64,
    status_429: u64,
    status_5xx: u64,
    thinking_requests: u64,
    effort_requests: u64,
    session_id_present: u64,
    client_request_id_present: u64,
    models: BTreeMap<String, u64>,
    entrypoints: BTreeMap<String, u64>,
    client_types: BTreeMap<String, u64>,
    betas: BTreeMap<String, u64>,
    telemetry_events: BTreeMap<String, u64>,
    telemetry_failed: u64,
    mismatches: BTreeMap<FingerprintMismatch, u64>,
}

#[derive(Debug, Serialize)]
struct ProfileSnapshotRecord<'a> {
    schema_version: u32,
    event: &'static str,
    timestamp: DateTime<Utc>,
    account_id: i64,
    auth_type: &'a str,
    auto_telemetry: bool,
    profile: &'a AuditProfile,
}

#[derive(Debug, Serialize)]
struct AnomalyRecord<'a> {
    schema_version: u32,
    event: &'static str,
    timestamp: DateTime<Utc>,
    account_id: i64,
    kind: FingerprintMismatch,
    profile: &'a AuditProfile,
}

#[derive(Debug, Serialize)]
struct SessionRecord<'a> {
    schema_version: u32,
    event: &'static str,
    timestamp: DateTime<Utc>,
    account_id: i64,
    state: &'static str,
    profile: &'a AuditProfile,
}

#[derive(Debug, Serialize)]
struct HourlySummaryRecord<'a> {
    schema_version: u32,
    event: &'static str,
    timestamp: DateTime<Utc>,
    account_id: i64,
    auth_type: &'a str,
    auto_telemetry: bool,
    profile: Option<&'a AuditProfile>,
    requests: u64,
    stream_requests: u64,
    status_2xx: u64,
    status_403: u64,
    status_429: u64,
    status_5xx: u64,
    thinking_requests: u64,
    effort_requests: u64,
    session_id_present: u64,
    client_request_id_present: u64,
    models: &'a BTreeMap<String, u64>,
    entrypoints: &'a BTreeMap<String, u64>,
    client_types: &'a BTreeMap<String, u64>,
    betas: &'a BTreeMap<String, u64>,
    telemetry_events: &'a BTreeMap<String, u64>,
    telemetry_failed: u64,
    mismatches: &'a BTreeMap<FingerprintMismatch, u64>,
    dropped_audit_observations: u64,
}

pub struct FingerprintAudit {
    sender: Option<mpsc::Sender<AuditMessage>>,
    dropped: Arc<AtomicU64>,
}

impl FingerprintAudit {
    pub fn start(enabled: bool, log_dir: &str) -> Arc<Self> {
        if !enabled {
            return Arc::new(Self {
                sender: None,
                dropped: Arc::new(AtomicU64::new(0)),
            });
        }

        let (sender, receiver) = mpsc::channel(AUDIT_QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let worker_dropped = dropped.clone();
        let path = Path::new(log_dir).to_path_buf();
        tokio::spawn(async move {
            audit_worker(receiver, worker_dropped, path, AUDIT_FLUSH_INTERVAL).await;
        });
        Arc::new(Self {
            sender: Some(sender),
            dropped,
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.sender.is_some()
    }

    pub fn observe_request(&self, observation: AuditRequestObservation) {
        self.try_send(AuditMessage::Request(observation));
    }

    pub fn observe_response(&self, account_id: i64, status: u16) {
        self.try_send(AuditMessage::Response { account_id, status });
    }

    pub fn observe_telemetry(&self, account_id: i64, event_name: &'static str, success: bool) {
        self.try_send(AuditMessage::Telemetry {
            account_id,
            event_name,
            success,
        });
    }

    pub fn observe_session(&self, account: &Account, state: &'static str) {
        let env = crate::model::identity::parse_canonical_env(&account.canonical_env);
        self.try_send(AuditMessage::Session {
            account_id: account.id,
            state,
            profile: AuditProfile::from_env(&env),
        });
    }

    fn try_send(&self, message: AuditMessage) {
        let Some(sender) = &self.sender else {
            return;
        };
        if sender.try_send(message).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct AuditAppender {
    writer: BasicRollingFileAppender,
    path: PathBuf,
}

async fn audit_worker(
    mut receiver: mpsc::Receiver<AuditMessage>,
    dropped: Arc<AtomicU64>,
    log_dir: PathBuf,
    flush_interval: Duration,
) {
    let mut appender = match create_audit_appender(&log_dir) {
        Ok(appender) => Some(appender),
        Err(error) => {
            warn!("fingerprint audit disabled: {}", error);
            None
        }
    };
    let mut counters: HashMap<i64, AccountCounters> = HashMap::new();
    let mut interval = tokio::time::interval(flush_interval);
    interval.tick().await;

    loop {
        tokio::select! {
            maybe = receiver.recv() => {
                let Some(message) = maybe else {
                    flush_summaries(&mut appender, &mut counters, dropped.swap(0, Ordering::Relaxed));
                    break;
                };
                apply_message(&mut appender, &mut counters, message);
            }
            _ = interval.tick() => {
                flush_summaries(&mut appender, &mut counters, dropped.swap(0, Ordering::Relaxed));
            }
        }
    }
}

fn apply_message(
    appender: &mut Option<AuditAppender>,
    counters: &mut HashMap<i64, AccountCounters>,
    message: AuditMessage,
) {
    match message {
        AuditMessage::Request(observation) => {
            let counter = counters.entry(observation.account_id).or_default();
            let profile_changed = counter.profile.as_ref() != Some(&observation.profile);
            counter.auth_type = observation.auth_type;
            counter.auto_telemetry = observation.auto_telemetry;
            counter.requests += 1;
            counter.stream_requests += u64::from(observation.stream);
            counter.thinking_requests += u64::from(observation.thinking.is_some());
            counter.effort_requests += u64::from(observation.effort.is_some());
            counter.session_id_present += u64::from(observation.session_id_present);
            counter.client_request_id_present += u64::from(observation.client_request_id_present);
            *counter.models.entry(observation.model).or_default() += 1;
            *counter
                .entrypoints
                .entry(observation.entrypoint)
                .or_default() += 1;
            *counter
                .client_types
                .entry(observation.client_type)
                .or_default() += 1;
            for beta in observation.betas {
                *counter.betas.entry(beta).or_default() += 1;
            }

            if profile_changed {
                counter.profile = Some(observation.profile.clone());
                let record = ProfileSnapshotRecord {
                    schema_version: AUDIT_SCHEMA_VERSION,
                    event: "profile_snapshot",
                    timestamp: Utc::now(),
                    account_id: observation.account_id,
                    auth_type: &counter.auth_type,
                    auto_telemetry: counter.auto_telemetry,
                    profile: &observation.profile,
                };
                write_record(appender, &record);
            }

            for mismatch in observation.mismatches {
                *counter.mismatches.entry(mismatch).or_default() += 1;
                let record = AnomalyRecord {
                    schema_version: AUDIT_SCHEMA_VERSION,
                    event: "fingerprint_anomaly",
                    timestamp: Utc::now(),
                    account_id: observation.account_id,
                    kind: mismatch,
                    profile: &observation.profile,
                };
                write_record(appender, &record);
            }
        }
        AuditMessage::Response { account_id, status } => {
            let counter = counters.entry(account_id).or_default();
            match status {
                200..=299 => counter.status_2xx += 1,
                403 => counter.status_403 += 1,
                429 => counter.status_429 += 1,
                500..=599 => counter.status_5xx += 1,
                _ => {}
            }
        }
        AuditMessage::Telemetry {
            account_id,
            event_name,
            success,
        } => {
            let counter = counters.entry(account_id).or_default();
            *counter
                .telemetry_events
                .entry(event_name.into())
                .or_default() += 1;
            if !success {
                counter.telemetry_failed += 1;
            }
        }
        AuditMessage::Session {
            account_id,
            state,
            profile,
        } => {
            let record = SessionRecord {
                schema_version: AUDIT_SCHEMA_VERSION,
                event: "telemetry_session",
                timestamp: Utc::now(),
                account_id,
                state,
                profile: &profile,
            };
            write_record(appender, &record);
        }
    }
}

fn flush_summaries(
    appender: &mut Option<AuditAppender>,
    counters: &mut HashMap<i64, AccountCounters>,
    dropped: u64,
) {
    let timestamp = Utc::now();
    for (account_id, counter) in counters.iter() {
        let record = HourlySummaryRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            event: "hourly_summary",
            timestamp,
            account_id: *account_id,
            auth_type: &counter.auth_type,
            auto_telemetry: counter.auto_telemetry,
            profile: counter.profile.as_ref(),
            requests: counter.requests,
            stream_requests: counter.stream_requests,
            status_2xx: counter.status_2xx,
            status_403: counter.status_403,
            status_429: counter.status_429,
            status_5xx: counter.status_5xx,
            thinking_requests: counter.thinking_requests,
            effort_requests: counter.effort_requests,
            session_id_present: counter.session_id_present,
            client_request_id_present: counter.client_request_id_present,
            models: &counter.models,
            entrypoints: &counter.entrypoints,
            client_types: &counter.client_types,
            betas: &counter.betas,
            telemetry_events: &counter.telemetry_events,
            telemetry_failed: counter.telemetry_failed,
            mismatches: &counter.mismatches,
            dropped_audit_observations: dropped,
        };
        write_record(appender, &record);
    }
    counters.clear();
}

fn create_audit_appender(log_dir: &Path) -> std::io::Result<AuditAppender> {
    std::fs::create_dir_all(log_dir)?;
    let path = log_dir.join(AUDIT_FILE_NAME);
    OpenOptions::new().create(true).append(true).open(&path)?;
    set_private_permissions(&path)?;
    let writer = BasicRollingFileAppender::new(
        &path,
        RollingConditionBasic::new().max_size(AUDIT_MAX_SIZE_BYTES),
        AUDIT_MAX_HISTORY_FILES,
    )?;
    Ok(AuditAppender { writer, path })
}

fn write_record<T: Serialize>(appender: &mut Option<AuditAppender>, record: &T) {
    let Some(appender) = appender.as_mut() else {
        return;
    };
    let result = serde_json::to_writer(&mut appender.writer, record)
        .map_err(std::io::Error::other)
        .and_then(|_| appender.writer.write_all(b"\n"))
        .and_then(|_| appender.writer.flush());
    match result {
        Ok(()) => {
            let _ = set_private_permissions(&appender.path);
        }
        Err(error) => warn!("fingerprint audit write failed: {}", error),
    }
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn safe_token(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-[]".contains(&byte))
    {
        value.to_string()
    } else {
        "other".into()
    }
}

fn safe_entrypoint(value: &str) -> String {
    match value {
        "cli"
        | "sdk-cli"
        | "sdk-ts"
        | "sdk-py"
        | "claude-vscode"
        | "remote"
        | "remote_baku"
        | "remote_cowork"
        | "remote_desktop"
        | "remote_mobile"
        | "mcp"
        | "local-agent"
        | "claude-code-github-action"
        | "claude_in_slack"
        | "claude-in-slack" => value.to_string(),
        _ => "other".into(),
    }
}

fn safe_optional_feature(value: Option<&str>, allowed: &[&str]) -> Option<String> {
    value.map(|value| {
        if allowed.contains(&value) {
            value.to_string()
        } else {
            "other".into()
        }
    })
}

pub fn request_observation(
    account: &Account,
    client_type: &str,
    entrypoint: &str,
    model: &str,
    stream: bool,
    thinking: Option<&str>,
    effort: Option<&str>,
    betas: Vec<String>,
    session_id_present: bool,
    client_request_id_present: bool,
    mismatches: Vec<FingerprintMismatch>,
) -> AuditRequestObservation {
    let env = crate::model::identity::parse_canonical_env(&account.canonical_env);
    AuditRequestObservation {
        account_id: account.id,
        auth_type: account.auth_type.to_string(),
        auto_telemetry: account.auto_telemetry,
        profile: AuditProfile::from_env(&env),
        client_type: match client_type {
            "claude_code" => "claude_code",
            "api" => "api",
            _ => "other",
        }
        .into(),
        entrypoint: safe_entrypoint(entrypoint),
        model: safe_token(model),
        stream,
        thinking: safe_optional_feature(thinking, &["adaptive", "enabled"]),
        effort: safe_optional_feature(effort, &["low", "medium", "high", "xhigh", "max"]),
        betas: betas.into_iter().map(|beta| safe_token(&beta)).collect(),
        session_id_present,
        client_request_id_present,
        mismatches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_record_schema_cannot_hold_sensitive_payloads() {
        let profile = AuditProfile {
            claude_version: "2.1.258".into(),
            build_time: "2026-09-01T21:54:40Z".into(),
            stainless_package: "0.112.1".into(),
            platform: "darwin".into(),
            arch: "arm64".into(),
            stainless_os: "MacOS".into(),
            runtime: "node".into(),
            runtime_version: "v26.3.0".into(),
            is_running_with_bun: true,
        };
        let record = AnomalyRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            event: "fingerprint_anomaly",
            timestamp: Utc::now(),
            account_id: 1,
            kind: FingerprintMismatch::Version,
            profile: &profile,
        };
        let json = serde_json::to_string(&record).unwrap();
        for forbidden in [
            "authorization",
            "refresh_token",
            "access_token",
            "email",
            "prompt",
            "response",
            "cookie",
            "proxy_url",
            "organization_uuid",
            "session_id\"",
            "request_id\"",
        ] {
            assert!(!json.to_ascii_lowercase().contains(forbidden), "{json}");
        }
    }

    #[test]
    fn arbitrary_request_values_are_normalized_before_audit() {
        assert_eq!(safe_token("secret with spaces"), "other");
        assert_eq!(safe_entrypoint("token=secret"), "other");
        assert_eq!(
            safe_optional_feature(Some("secret"), &["adaptive", "enabled"]).as_deref(),
            Some("other")
        );
    }

    #[test]
    fn full_queue_drops_without_blocking() {
        let (sender, _receiver) = mpsc::channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let audit = FingerprintAudit {
            sender: Some(sender),
            dropped: dropped.clone(),
        };
        audit.observe_response(1, 200);
        audit.observe_response(1, 200);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn writer_emits_private_ndjson() {
        let dir = std::env::temp_dir().join(format!("audit-writer-{}", uuid::Uuid::new_v4()));
        let mut appender = Some(create_audit_appender(&dir).unwrap());
        let profile = AuditProfile {
            claude_version: "2.1.258".into(),
            build_time: "2026-09-01T21:54:40Z".into(),
            stainless_package: "0.112.1".into(),
            platform: "darwin".into(),
            arch: "arm64".into(),
            stainless_os: "MacOS".into(),
            runtime: "node".into(),
            runtime_version: "v26.3.0".into(),
            is_running_with_bun: true,
        };
        let record = SessionRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            event: "telemetry_session",
            timestamp: Utc::now(),
            account_id: 1,
            state: "started",
            profile: &profile,
        };
        write_record(&mut appender, &record);
        drop(appender);
        let path = dir.join(AUDIT_FILE_NAME);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(serde_json::from_str::<serde_json::Value>(text.trim()).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn disabled_audit_does_not_create_file() {
        let dir = std::env::temp_dir().join(format!("audit-disabled-{}", uuid::Uuid::new_v4()));
        let audit = FingerprintAudit::start(false, dir.to_str().unwrap());
        assert!(!audit.is_enabled());
        assert!(!dir.join(AUDIT_FILE_NAME).exists());
    }
}
