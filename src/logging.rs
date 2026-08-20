use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::path::{Path, PathBuf};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const LOG_FILE_NAME: &str = "cc-bridge.log";
const LOG_MAX_SIZE_BYTES: u64 = 30 * 1024 * 1024;
// rolling-file 的参数是历史文件数；加上 active file 后总数最多 10。
const LOG_MAX_HISTORY_FILES: usize = 9;

pub fn init(log_level: &str, log_dir: &str) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| log_level.into());

    match create_appender(
        Path::new(log_dir),
        LOG_MAX_SIZE_BYTES,
        LOG_MAX_HISTORY_FILES,
    ) {
        Ok(appender) => {
            let (file_writer, guard) =
                tracing_appender::non_blocking::NonBlockingBuilder::default()
                    .lossy(false)
                    .finish(appender);
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(file_writer),
                )
                .init();
            Some(guard)
        }
        Err(error) => {
            eprintln!(
                "file logging disabled: cannot initialize {}/{}: {}",
                log_dir, LOG_FILE_NAME, error
            );
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .init();
            None
        }
    }
}

fn create_appender(
    log_dir: &Path,
    max_size_bytes: u64,
    max_history_files: usize,
) -> std::io::Result<BasicRollingFileAppender> {
    std::fs::create_dir_all(log_dir)?;
    let path: PathBuf = log_dir.join(LOG_FILE_NAME);
    BasicRollingFileAppender::new(
        path,
        RollingConditionBasic::new().max_size(max_size_bytes),
        max_history_files,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rolling_file_keeps_active_plus_configured_history_count() {
        let dir = std::env::temp_dir().join(format!("cc-bridge-log-test-{}", uuid::Uuid::new_v4()));
        let mut appender = create_appender(&dir, 64, 2).expect("create appender");

        for index in 0..12 {
            writeln!(appender, "entry-{index:02}-abcdefghijklmnopqrstuvwxyz").expect("write log");
        }
        appender.flush().expect("flush log");

        let file_count = std::fs::read_dir(&dir)
            .expect("read log dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(LOG_FILE_NAME)
            })
            .count();
        assert!(
            file_count <= 3,
            "active + 2 history expected, got {file_count}"
        );

        drop(appender);
        let _ = std::fs::remove_dir_all(dir);
    }
}
