use std::{
    fmt as stdfmt,
    fs::{self, File},
    path::{Path, PathBuf},
};

use chrono::Local;
use rmcp::{RoleServer, service::RequestContext};
use tracing::{Event, Subscriber, field::Visit};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter,
    fmt::{
        FmtContext,
        format::{FormatEvent, FormatFields, Writer},
        layer,
    },
    layer::SubscriberExt,
    registry::{LookupSpan, Registry},
    util::SubscriberInitExt,
};

const DEBUG_LOG_FILE: &str = "debug_log.txt";

pub fn default_debug_log_path() -> std::io::Result<PathBuf> {
    Ok(std::env::current_dir()?.join(DEBUG_LOG_FILE))
}

pub fn prepare_debug_log_file(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    File::create(path)?;
    Ok(())
}

pub fn init_debug_logging(
    log_path: &Path,
    env_filter: EnvFilter,
) -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    prepare_debug_log_file(log_path)?;

    let file_appender = tracing_appender::rolling::never(
        log_path.parent().unwrap_or_else(|| Path::new(".")),
        log_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(DEBUG_LOG_FILE)),
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let fmt_layer = layer()
        .with_ansi(false)
        .event_format(DebugLogFormatter)
        .with_writer(non_blocking);

    Registry::default().with(env_filter).with(fmt_layer).init();

    Ok(guard)
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

struct DebugLogFormatter;

impl<S, N> FormatEvent<S, N> for DebugLogFormatter
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> stdfmt::Result {
        let mut visitor = FieldCollector::default();
        event.record(&mut visitor);

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let level = visitor
            .phase
            .as_deref()
            .unwrap_or_else(|| event.metadata().level().as_str());
        let context_id = visitor.context_id.as_deref().unwrap_or("system");

        write!(writer, "[{timestamp}] [{level}] [{context_id}] ")?;

        let scope = ctx
            .event_scope()
            .map(|scope| {
                scope
                    .from_root()
                    .map(|span| span.metadata().name())
                    .collect::<Vec<_>>()
                    .join(" → ")
            })
            .unwrap_or_default();

        let mut prefix = if scope.is_empty() {
            event.metadata().target().to_string()
        } else {
            scope
        };

        if let Some(line) = event.metadata().line() {
            if prefix.is_empty() {
                prefix = format!("line {line}");
            } else {
                prefix = format!("{prefix} line {line}");
            }
        }

        if !prefix.is_empty() {
            write!(writer, "{prefix}: ")?;
        }

        if let Some(message) = visitor.message.take() {
            write!(writer, "{message}")?;
        } else {
            write!(writer, "event")?;
        }

        if !visitor.extra_fields.is_empty() {
            write!(writer, " | {}", visitor.extra_fields.join(" | "))?;
        }

        writeln!(writer)
    }
}

#[derive(Default)]
struct FieldCollector {
    context_id: Option<String>,
    message: Option<String>,
    phase: Option<String>,
    extra_fields: Vec<String>,
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn stdfmt::Debug) {
        let rendered = sanitize_log_value(&format!("{value:?}"));
        match field.name() {
            "message" => self.message = Some(rendered.trim_matches('"').to_string()),
            "context_id" => self.context_id = Some(rendered.trim_matches('"').to_string()),
            "phase" => self.phase = Some(rendered.trim_matches('"').to_string()),
            _ => self
                .extra_fields
                .push(format!("{}={}", field.name(), rendered)),
        }
    }
}
