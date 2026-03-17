use std::{
    fs,
    path::{Path, PathBuf},
};

use rmcp::{RoleServer, service::RequestContext};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    fmt::layer,
    layer::SubscriberExt,
    registry::Registry,
    util::SubscriberInitExt,
    EnvFilter,
};

/// Initialize logging - to stderr by default, or to file if log_path is specified
pub fn init_logging(
    log_path: Option<&PathBuf>,
    log_level: &str,
) -> Result<Option<WorkerGuard>, Box<dyn std::error::Error>> {
    let env_filter = EnvFilter::from_default_env()
        .add_directive(log_level.parse()?);

    match log_path {
        Some(path) => {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            let file_appender = tracing_appender::rolling::never(
                path.parent().unwrap_or_else(|| Path::new(".")),
                path.file_name().unwrap_or_else(|| std::ffi::OsStr::new("nvim-mcp.log")),
            );
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            let fmt_layer = layer()
                .with_ansi(false)
                .with_writer(non_blocking);

            Registry::default().with(env_filter).with(fmt_layer).init();

            Ok(Some(guard))
        }
        None => {
            // Default: log to stderr
            let fmt_layer = layer()
                .with_ansi(true);

            Registry::default().with(env_filter).with(fmt_layer).init();

            Ok(None)
        }
    }
}

pub fn sanitize_log_value(value: &str) -> String {
    value.replace('\n', "\\n").replace('\r', "\\r")
}

pub fn preview_text(value: &str, limit: usize) -> String {
    let sanitized = sanitize_log_value(value.trim());
    if sanitized.chars().count() <= limit {
        sanitized
    } else {
        let prefix: String = sanitized.chars().take(limit).collect();
        format!("{prefix}...")
    }
}

pub fn preview_json(value: &serde_json::Value, limit: usize) -> String {
    preview_text(&value.to_string(), limit)
}

pub fn connection_context_id(connection_id: &str, operation: &str) -> String {
    format!("{operation}:{connection_id}")
}

pub fn request_context_id(ctx: &RequestContext<RoleServer>, fallback: &str) -> String {
    if let Some(tool_use_id) = ctx
        .meta
        .get("claudecode/toolUseId")
        .and_then(|value| value.as_str())
    {
        return tool_use_id.to_string();
    }

    if let Some(progress_token) = ctx.meta.get("progressToken") {
        return format!(
            "progress_{}",
            sanitize_log_value(&progress_token.to_string())
        );
    }

    format!("{fallback}_{:?}", ctx.id)
}
