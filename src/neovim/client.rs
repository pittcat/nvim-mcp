#![allow(rustdoc::invalid_codeblock_attributes)]

use std::collections::HashMap;
use std::fmt::{self, Display};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use nvim_rs::{Handler, Neovim, create::tokio as create};
use rmpv::Value;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use tokio::{
    io::AsyncWrite,
    net::TcpStream,
    sync::Mutex,
    time::{Duration, timeout},
};
use tracing::{debug, info, warn, instrument};

use super::{connection::NeovimConnection, error::NeovimError};
use crate::logging::preview_text;

/// Common trait for Neovim client operations
#[async_trait]
pub trait NeovimClientTrait: Sync + Send {
    /// Get the target of the Neovim connection
    fn target(&self) -> Option<String>;

    /// Check if the connection is still alive (synchronous check)
    fn is_alive(&self) -> bool;

    /// Disconnect from the current Neovim instance
    async fn disconnect(&mut self) -> Result<String, NeovimError>;

    /// Get information about all buffers
    async fn get_buffers(&self) -> Result<Vec<BufferInfo>, NeovimError>;

    /// Execute Lua code in Neovim
    async fn execute_lua(&self, code: &str) -> Result<Value, NeovimError>;

    /// Wait for a specific notification with timeout
    async fn wait_for_notification(
        &self,
        notification_name: &str,
        timeout_ms: u64,
    ) -> Result<Notification, NeovimError>;

    /// Navigate to a specific position in a document
    async fn navigate(
        &self,
        document: DocumentIdentifier,
        position: Position,
    ) -> Result<NavigateResult, NeovimError>;

    /// Read document content by DocumentIdentifier with optional line range
    async fn read_document(
        &self,
        document: DocumentIdentifier,
        start: i64,
        end: i64,
    ) -> Result<String, NeovimError>;
}

/// Notification tracking structure
#[derive(Debug, Clone)]
pub struct Notification {
    pub name: String,
    pub args: Vec<Value>,
    pub timestamp: std::time::SystemTime,
}

/// Shared state for notification tracking
#[derive(Clone, Default)]
pub struct NotificationTracker {
    notifications: Arc<Mutex<Vec<Notification>>>,
    notify_wakers: Arc<Mutex<HashMap<String, Vec<tokio::sync::oneshot::Sender<Notification>>>>>,
}

/// Configuration for notification cleanup
const MAX_STORED_NOTIFICATIONS: usize = 100;
const NOTIFICATION_EXPIRY_SECONDS: u64 = 30;

impl NotificationTracker {
    /// Clean up expired and excess notifications
    async fn cleanup_notifications(&self) {
        let mut notifications = self.notifications.lock().await;

        // Remove expired notifications
        let now = std::time::SystemTime::now();
        notifications.retain(|n| {
            now.duration_since(n.timestamp)
                .map(|d| d.as_secs() < NOTIFICATION_EXPIRY_SECONDS)
                .unwrap_or(false)
        });

        // If still too many notifications, keep only the most recent ones
        if notifications.len() > MAX_STORED_NOTIFICATIONS {
            let excess = notifications.len() - MAX_STORED_NOTIFICATIONS;
            notifications.drain(0..excess);
        }
    }

    /// Record a notification
    pub async fn record_notification(&self, name: String, args: Vec<Value>) {
        let notification = Notification {
            name: name.clone(),
            args,
            timestamp: std::time::SystemTime::now(),
        };

        // Notify any waiting tasks for this specific notification name first
        let mut wakers = self.notify_wakers.lock().await;
        if let Some(waiters) = wakers.get_mut(&name) {
            while let Some(waker) = waiters.pop() {
                let _ = waker.send(notification.clone());
            }
        }

        // Clean up wakers with no waiters
        wakers.retain(|_, waiters| !waiters.is_empty());
        drop(wakers); // Release lock early

        // Always store recent notifications for potential future requests
        // but clean up old/excess ones to prevent memory leaks
        {
            let mut notifications = self.notifications.lock().await;
            notifications.push(notification);

            // Trigger cleanup if we're approaching the limit
            if notifications.len() > MAX_STORED_NOTIFICATIONS * 3 / 4 {
                drop(notifications); // Release lock before calling cleanup
                self.cleanup_notifications().await;
            }
        }
    }

    /// Wait for a specific notification with timeout
    pub async fn wait_for_notification(
        &self,
        notification_name: &str,
        timeout_duration: Duration,
    ) -> Result<Notification, NeovimError> {
        // First check if a recent (non-expired) notification already exists
        {
            let notifications = self.notifications.lock().await;
            let now = std::time::SystemTime::now();

            if let Some(notification) = notifications
                .iter()
                .rev() // Check most recent first
                .find(|n| {
                    n.name == notification_name
                        && now
                            .duration_since(n.timestamp)
                            .map(|d| d.as_secs() < NOTIFICATION_EXPIRY_SECONDS)
                            .unwrap_or(false)
                })
            {
                return Ok(notification.clone());
            }
        }

        // Create a oneshot channel to wait for the notification
        let (tx, rx) = tokio::sync::oneshot::channel();

        // Register our interest in this notification
        let mut wakers = self.notify_wakers.lock().await;
        wakers
            .entry(notification_name.to_string())
            .or_insert_with(Vec::new)
            .push(tx);

        // Wait for the notification with timeout
        drop(wakers); // Release the lock before awaiting

        match timeout(timeout_duration, rx).await {
            Ok(Ok(notification)) => Ok(notification),
            Ok(Err(_)) => Err(NeovimError::Api(
                "Notification channel closed unexpectedly".to_string(),
            )),
            Err(_) => Err(NeovimError::Api(format!(
                "Timeout waiting for notification: {}",
                notification_name
            ))),
        }
    }

    /// Clear all recorded notifications
    pub async fn clear_notifications(&self) {
        let mut notifications = self.notifications.lock().await;
        notifications.clear();
    }

    /// Manually trigger cleanup of expired notifications
    #[allow(dead_code)]
    pub(crate) async fn cleanup_expired_notifications(&self) {
        self.cleanup_notifications().await;
    }

    /// Get current notification statistics (for debugging/monitoring)
    #[allow(dead_code)]
    pub(crate) async fn get_stats(&self) -> (usize, usize) {
        let notifications = self.notifications.lock().await;
        let wakers = self.notify_wakers.lock().await;
        (notifications.len(), wakers.len())
    }
}

pub struct NeovimHandler<T> {
    _marker: std::marker::PhantomData<T>,
    notification_tracker: NotificationTracker,
}

impl<T> NeovimHandler<T> {
    pub fn new() -> Self {
        NeovimHandler {
            _marker: std::marker::PhantomData,
            notification_tracker: NotificationTracker::default(),
        }
    }

    pub fn notification_tracker(&self) -> NotificationTracker {
        self.notification_tracker.clone()
    }
}

impl<T> Clone for NeovimHandler<T> {
    fn clone(&self) -> Self {
        NeovimHandler {
            _marker: std::marker::PhantomData,
            notification_tracker: self.notification_tracker.clone(),
        }
    }
}

#[async_trait]
impl<T> Handler for NeovimHandler<T>
where
    T: futures::AsyncWrite + Send + Sync + Unpin + 'static,
{
    type Writer = T;

    async fn handle_notify(&self, name: String, args: Vec<Value>, _neovim: Neovim<T>) {
        info!("handling notification: {name:?}, {args:?}");
        self.notification_tracker
            .record_notification(name, args)
            .await;
    }

    async fn handle_request(
        &self,
        name: String,
        args: Vec<Value>,
        _neovim: Neovim<T>,
    ) -> Result<Value, Value> {
        info!("handling request: {name:?}, {args:?}");
        match name.as_ref() {
            "ping" => Ok(Value::from("pong")),
            _ => Ok(Value::Nil),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BufferInfo {
    pub id: u64,
    pub name: String,
    pub line_count: u64,
}

/// Text documents are identified using a URI.
/// On the protocol level, URIs are passed as strings.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct TextDocumentIdentifier {
    /// The text document's URI.
    uri: String,
    /// The version number of this document. If an optional versioned text document
    /// identifier is sent from the server to the client and the file is not
    /// open in the editor (the server has not received an open notification
    /// before) the server can send `null` to indicate that the version is
    /// known and the content on disk is the master (as specified with document
    /// content ownership).
    ///
    /// The version number of a document will increase after each change,
    /// including undo/redo. The number doesn't need to be consecutive.
    version: Option<i32>,
}

/// This is a Visitor that forwards string types to T's `FromStr` impl and
/// forwards map types to T's `Deserialize` impl. The `PhantomData` is to
/// keep the compiler from complaining about T being an unused generic type
/// parameter. We need T in order to know the Value type for the Visitor
/// impl.
struct StringOrStruct<T>(PhantomData<fn() -> T>);

impl<'de, T> Visitor<'de> for StringOrStruct<T>
where
    T: Deserialize<'de> + FromStr,
    <T as FromStr>::Err: Display,
{
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("string or map")
    }

    fn visit_str<E>(self, value: &str) -> Result<T, E>
    where
        E: de::Error,
    {
        FromStr::from_str(value).map_err(de::Error::custom)
    }

    fn visit_map<M>(self, map: M) -> Result<T, M::Error>
    where
        M: MapAccess<'de>,
    {
        // `MapAccessDeserializer` is a wrapper that turns a `MapAccess`
        // into a `Deserializer`, allowing it to be used as the input to T's
        // `Deserialize` implementation. T then deserializes itself using
        // the entries from the map visitor.
        Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))
    }
}

/// Custom deserializer function that handles both formats
pub fn string_or_struct<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + FromStr,
    <T as FromStr>::Err: Display,
{
    deserializer.deserialize_any(StringOrStruct(PhantomData))
}

/// Universal identifier for text documents supporting multiple reference types
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentIdentifier {
    /// Reference by Neovim buffer ID (for currently open files)
    BufferId(u64),
    /// Reference by project-relative path
    ProjectRelativePath(PathBuf),
    /// Reference by absolute file path
    AbsolutePath(PathBuf),
}

macro_rules! impl_fromstr_serde_json {
    ($type:ty) => {
        impl FromStr for $type {
            type Err = serde_json::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                serde_json::from_str(s)
            }
        }
    };
}

impl_fromstr_serde_json!(DocumentIdentifier);

impl DocumentIdentifier {
    /// Create from buffer ID
    pub fn from_buffer_id(buffer_id: u64) -> Self {
        Self::BufferId(buffer_id)
    }

    /// Create from project-relative path
    pub fn from_project_path<P: Into<PathBuf>>(path: P) -> Self {
        Self::ProjectRelativePath(path.into())
    }

    /// Create from absolute path
    pub fn from_absolute_path<P: Into<PathBuf>>(path: P) -> Self {
        Self::AbsolutePath(path.into())
    }
}

/// Position in a text document expressed as zero-based line and zero-based character offset.
/// A position is between two characters like an 'insert' cursor in an editor.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct Position {
    /// Line position in a document (zero-based).
    pub line: u64,
    /// Character offset on a line in a document (zero-based).
    pub character: u64,
}

/// Result of a navigate operation
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct NavigateResult {
    /// The path of the file to open
    pub path: String,
    /// The line number (0-based)
    pub line: u64,
    /// The column number (0-based)
    pub column: u64,
}

/// Parameters for text document position requests
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentPositionParams {
    /// The text document identifier
    pub text_document: TextDocumentIdentifier,
    /// The position in the text document
    pub position: Position,
}

/// Read document parameters
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ReadDocumentParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
}

impl ReadDocumentParams {
    fn buffer_id(id: u64, start: i64, end: i64) -> Self {
        Self {
            buffer_id: Some(id),
            file_path: None,
            start_line: start,
            end_line: end,
        }
    }

    fn path<P: AsRef<Path>>(path: P, start: i64, end: i64) -> Self {
        Self {
            buffer_id: None,
            file_path: Some(path.as_ref().to_string_lossy().to_string()),
            start_line: start,
            end_line: end,
        }
    }
}

/// Configuration for Neovim client operations
#[derive(Debug, Clone, Default)]
pub struct NeovimClientConfig {}

pub struct NeovimClient<T>
where
    T: AsyncWrite + Send + 'static,
{
    connection: Option<NeovimConnection<T>>,
    notification_tracker: Option<NotificationTracker>,
    config: NeovimClientConfig,
}

impl<T> Default for NeovimClient<T>
where
    T: AsyncWrite + Send + 'static,
{
    fn default() -> Self {
        Self {
            connection: None,
            notification_tracker: None,
            config: NeovimClientConfig::default(),
        }
    }
}

#[cfg(unix)]
type Connection = tokio::net::UnixStream;
#[cfg(windows)]
type Connection = tokio::net::windows::named_pipe::NamedPipeClient;

/// Creates a TextDocumentIdentifier from a file path
/// This utility function works independently of Neovim buffers
#[allow(dead_code)]
pub fn make_text_document_identifier_from_path<P: AsRef<Path>>(
    file_path: P,
) -> Result<TextDocumentIdentifier, NeovimError> {
    let path = file_path.as_ref();

    // Convert to absolute path and canonicalize
    let absolute_path = path.canonicalize().map_err(|e| {
        NeovimError::Api(format!("Failed to resolve path {}: {}", path.display(), e))
    })?;

    // Convert to file:// URI
    let uri = format!("file://{}", absolute_path.display());

    Ok(TextDocumentIdentifier {
        uri,
        version: None, // No version for path-based identifiers
    })
}

/// Nvim execute_lua custom result type
#[derive(Debug, serde::Deserialize)]
pub enum NvimExecuteLuaResult<T> {
    #[serde(rename = "err_msg")]
    Error(String),
    #[serde(rename = "result")]
    Ok(T),
}

impl<T> From<NvimExecuteLuaResult<T>> for Result<T, NeovimError> {
    fn from(val: NvimExecuteLuaResult<T>) -> Self {
        use NvimExecuteLuaResult::*;
        match val {
            Ok(result) => Result::Ok(result),
            Error(msg) => Err(NeovimError::Api(msg)),
        }
    }
}

impl NeovimClient<Connection> {
    #[instrument(skip(self))]
    pub async fn connect_path(&mut self, path: &str) -> Result<(), NeovimError> {
        if let Some(ref conn) = self.connection {
            return Err(NeovimError::Connection(format!(
                "Already connected to {}. Disconnect first.",
                conn.target()
            )));
        }

        debug!("Attempting to connect to Neovim at {}", path);
        info!(context_id = format!("sock:{}", preview_text(path, 48)), "Connecting via socket to Neovim");
        let handler = NeovimHandler::new();
        let notification_tracker = handler.notification_tracker();
        match create::new_path(path, handler).await {
            Ok((nvim, io_handler)) => {
                let connection = NeovimConnection::new(
                    nvim,
                    tokio::spawn(async move {
                        let rv = io_handler.await;
                        debug!("io_handler completed with result: {:?}", rv);
                        rv
                    }),
                    path.to_string(),
                );
                self.connection = Some(connection);
                self.notification_tracker = Some(notification_tracker);
                debug!("Successfully connected to Neovim at {}", path);
                info!(context_id = format!("sock:{}", preview_text(path, 48)), "Socket connection established");
                Ok(())
            }
            Err(e) => {
                debug!("Failed to connect to Neovim at {}: {}", path, e);
                warn!(context_id = format!("sock:{}", preview_text(path, 48)), "Socket connection failed: {}", e);
                Err(NeovimError::Connection(format!("Connection failed: {e}")))
            }
        }
    }
}

impl NeovimClient<TcpStream> {
    #[instrument(skip(self))]
    pub async fn connect_tcp(&mut self, address: &str) -> Result<(), NeovimError> {
        if let Some(ref conn) = self.connection {
            return Err(NeovimError::Connection(format!(
                "Already connected to {}. Disconnect first.",
                conn.target()
            )));
        }

        debug!("Attempting to connect to Neovim at {}", address);
        info!(context_id = format!("tcp:{}", preview_text(address, 48)), "Connecting via TCP to Neovim");
        let handler = NeovimHandler::new();
        let notification_tracker = handler.notification_tracker();
        match create::new_tcp(address, handler).await {
            Ok((nvim, io_handler)) => {
                let connection = NeovimConnection::new(
                    nvim,
                    tokio::spawn(async move {
                        let rv = io_handler.await;
                        debug!("io_handler completed with result: {:?}", rv);
                        rv
                    }),
                    address.to_string(),
                );
                self.connection = Some(connection);
                self.notification_tracker = Some(notification_tracker);
                debug!("Successfully connected to Neovim at {}", address);
                info!(context_id = format!("tcp:{}", preview_text(address, 48)), "TCP connection established");
                Ok(())
            }
            Err(e) => {
                debug!("Failed to connect to Neovim at {}: {}", address, e);
                warn!(context_id = format!("tcp:{}", preview_text(address, 48)), "TCP connection failed: {}", e);
                Err(NeovimError::Connection(format!("Connection failed: {e}")))
            }
        }
    }
}

impl<T> NeovimClient<T>
where
    T: AsyncWrite + Send + 'static,
{
    /// Configure the Neovim client with custom settings
    #[allow(dead_code)]
    pub fn with_config(mut self, config: NeovimClientConfig) -> Self {
        self.config = config;
        self
    }

    /// Get project root directory from Neovim (working directory)
    #[instrument(skip(self))]
    async fn get_project_root(&self) -> Result<PathBuf, NeovimError> {
        let conn = self.connection.as_ref().ok_or_else(|| {
            NeovimError::Connection("Not connected to any Neovim instance".to_string())
        })?;

        match conn
            .nvim
            .execute_lua("return vim.fn.getcwd()", vec![])
            .await
        {
            Ok(value) => {
                let cwd = value.as_str().ok_or_else(|| {
                    NeovimError::Api("Invalid working directory format".to_string())
                })?;
                Ok(PathBuf::from(cwd))
            }
            Err(e) => Err(NeovimError::Api(format!(
                "Failed to get working directory: {e}"
            ))),
        }
    }

    /// Resolve DocumentIdentifier to TextDocumentIdentifier for buffer-based operations
    #[instrument(skip(self))]
    async fn resolve_text_document_identifier(
        &self,
        identifier: &DocumentIdentifier,
    ) -> Result<TextDocumentIdentifier, NeovimError> {
        let conn = self.connection.as_ref().ok_or_else(|| {
            NeovimError::Connection("Not connected to any Neovim instance".to_string())
        })?;

        match identifier {
            DocumentIdentifier::BufferId(buffer_id) => {
                // Get buffer URI directly from Neovim
                match conn
                    .nvim
                    .execute_lua(
                        "return vim.uri_from_bufnr(...)",
                        vec![Value::from(*buffer_id)],
                    )
                    .await
                {
                    Ok(uri) => {
                        let uri_str = uri.as_str().ok_or_else(|| {
                            NeovimError::Api("Invalid buffer URI format".to_string())
                        })?;
                        Ok(TextDocumentIdentifier {
                            uri: uri_str.to_string(),
                            version: None,
                        })
                    }
                    Err(e) => Err(NeovimError::Api(format!("Failed to get buffer URI: {e}"))),
                }
            }
            DocumentIdentifier::ProjectRelativePath(rel_path) => {
                // Get project root from Neovim
                let project_root = self.get_project_root().await?;
                let absolute_path = project_root.join(rel_path);
                make_text_document_identifier_from_path(absolute_path)
            }
            DocumentIdentifier::AbsolutePath(abs_path) => {
                // Use the existing path-based helper function
                make_text_document_identifier_from_path(abs_path)
            }
        }
    }
}

#[async_trait]
impl<T> NeovimClientTrait for NeovimClient<T>
where
    T: AsyncWrite + Send + 'static,
{
    fn target(&self) -> Option<String> {
        self.connection.as_ref().map(|c| c.target().to_string())
    }

    #[instrument(skip(self))]
    fn is_alive(&self) -> bool {
        // Check if connection exists and io_handler is still running
        if let Some(ref conn) = self.connection {
            !conn.io_handler.is_finished()
        } else {
            false
        }
    }

    #[instrument(skip(self))]
    async fn disconnect(&mut self) -> Result<String, NeovimError> {
        debug!("Attempting to disconnect from Neovim");
        let context_id = self
            .target()
            .map(|target| preview_text(&target, 48))
            .unwrap_or_else(|| "nvim-client".to_string());

        if let Some(connection) = self.connection.take() {
            let target = connection.target().to_string();
            connection.io_handler.abort();

            // Clear notification tracker to free memory
            if let Some(tracker) = self.notification_tracker.take() {
                tracker.clear_notifications().await;
            }

            debug!("Successfully disconnected from Neovim at {}", target);
            info!(context_id = context_id, "Disconnected from Neovim");
            Ok(target)
        } else {
            Err(NeovimError::Connection(
                "Not connected to any Neovim instance".to_string(),
            ))
        }
    }

    #[instrument(skip(self))]
    async fn get_buffers(&self) -> Result<Vec<BufferInfo>, NeovimError> {
        debug!("Getting buffer information");
        let context_id = self
            .target()
            .map(|target| preview_text(&target, 48))
            .unwrap_or_else(|| "nvim-client".to_string());

        // Use inline Lua to get buffer information
        let lua_code = r#"
            local buffers = {}
            for _, buf in ipairs(vim.api.nvim_list_bufs()) do
                if vim.api.nvim_buf_is_valid(buf) then
                    local name = vim.api.nvim_buf_get_name(buf)
                    local lines = vim.api.nvim_buf_line_count(buf)
                    table.insert(buffers, {
                        id = buf,
                        name = name,
                        line_count = lines
                    })
                end
            end
            return vim.json.encode(buffers)
        "#;

        match self.execute_lua(lua_code).await {
            Ok(buffers) => {
                debug!("Get buffers retrieved successfully");
                let buffers: Vec<BufferInfo> = match serde_json::from_str(buffers.as_str().unwrap())
                {
                    Ok(d) => d,
                    Err(e) => {
                        debug!("Failed to parse buffers: {}", e);
                        return Err(NeovimError::Api(format!("Failed to parse buffers: {e}")));
                    }
                };
                debug!(context_id = context_id, "Found {} buffers", buffers.len());
                Ok(buffers)
            }
            Err(e) => {
                debug!("Failed to get buffer info: {}", e);
                Err(NeovimError::Api(format!("Failed to get buffer info: {e}")))
            }
        }
    }

    #[instrument(skip(self))]
    async fn execute_lua(&self, code: &str) -> Result<Value, NeovimError> {
        let context_id = self
            .target()
            .map(|target| preview_text(&target, 48))
            .unwrap_or_else(|| "nvim-client".to_string());
        debug!(context_id = context_id.clone(), "Executing Lua code: {}", preview_text(code, 120));

        if code.trim().is_empty() {
            return Err(NeovimError::Api("Lua code cannot be empty".to_string()));
        }

        let conn = self.connection.as_ref().ok_or_else(|| {
            NeovimError::Connection("Not connected to any Neovim instance".to_string())
        })?;

        let lua_args = Vec::<Value>::new();
        match conn.nvim.exec_lua(code, lua_args).await {
            Ok(result) => {
                debug!(context_id = context_id, "Lua execution successful");
                Ok(result)
            }
            Err(e) => {
                debug!(context_id = context_id, "Lua execution failed: {e}");
                warn!(context_id = context_id, "Lua error: {}", e);
                Err(NeovimError::Api(format!("Lua execution failed: {e}")))
            }
        }
    }

    #[instrument(skip(self))]
    async fn wait_for_notification(
        &self,
        notification_name: &str,
        timeout_ms: u64,
    ) -> Result<Notification, NeovimError> {
        debug!(
            "Waiting for notification: {} with timeout: {}ms",
            notification_name, timeout_ms
        );

        let tracker = self.notification_tracker.as_ref().ok_or_else(|| {
            NeovimError::Connection("Not connected to any Neovim instance".to_string())
        })?;

        tracker
            .wait_for_notification(notification_name, Duration::from_millis(timeout_ms))
            .await
    }

    #[instrument(skip(self))]
    async fn navigate(
        &self,
        document: DocumentIdentifier,
        position: Position,
    ) -> Result<NavigateResult, NeovimError> {
        let context_id = self
            .target()
            .map(|target| preview_text(&target, 48))
            .unwrap_or_else(|| "nvim-client".to_string());
        let text_document = self.resolve_text_document_identifier(&document).await?;
        let _text_document_uri = text_document.uri.clone();
        let _target_line = position.line;
        let _target_character = position.character;

        let conn = self.connection.as_ref().ok_or_else(|| {
            NeovimError::Connection("Not connected to any Neovim instance".to_string())
        })?;

        match conn
            .nvim
            .execute_lua(
                include_str!("lua/navigate.lua"),
                vec![Value::from(
                    serde_json::to_string(&TextDocumentPositionParams {
                        text_document,
                        position,
                    })
                    .unwrap(),
                )],
            )
            .await
        {
            Ok(result) => {
                match serde_json::from_str::<NvimExecuteLuaResult<NavigateResult>>(
                    result.as_str().unwrap(),
                ) {
                    Ok(rv) => {
                        let output: Result<NavigateResult, NeovimError> = rv.into();
                        if let Ok(ref navigate_result) = output {
                            debug!(context_id = context_id, "Navigation complete: {}", preview_text(&navigate_result.path, 120));
                        }
                        output
                    }
                    Err(e) => {
                        debug!("Failed to parse navigate result: {}", e);
                        Err(NeovimError::Api(format!(
                            "Failed to parse navigate result: {e}"
                        )))
                    }
                }
            }
            Err(e) => {
                debug!("Failed to navigate: {}", e);
                Err(NeovimError::Api(format!("Failed to navigate: {e}")))
            }
        }
    }

    #[instrument(skip(self))]
    async fn read_document(
        &self,
        document: DocumentIdentifier,
        start: i64,
        end: i64,
    ) -> Result<String, NeovimError> {
        let context_id = self
            .target()
            .map(|target| preview_text(&target, 48))
            .unwrap_or_else(|| "nvim-client".to_string());
        let conn = self.connection.as_ref().ok_or_else(|| {
            NeovimError::Connection("Not connected to any Neovim instance".to_string())
        })?;
        let params = match &document {
            DocumentIdentifier::BufferId(buffer_id) => {
                ReadDocumentParams::buffer_id(*buffer_id, start, end)
            }
            DocumentIdentifier::ProjectRelativePath(rel_path) => {
                // Get project root and construct absolute path
                let project_root = self.get_project_root().await?;
                let absolute_path = project_root.join(rel_path);
                ReadDocumentParams::path(absolute_path, start, end)
            }
            DocumentIdentifier::AbsolutePath(abs_path) => {
                ReadDocumentParams::path(abs_path, start, end)
            }
        };
        match conn
            .nvim
            .execute_lua(
                include_str!("lua/read_document.lua"),
                vec![
                    Value::from(serde_json::to_string(&params).unwrap()), // params
                ],
            )
            .await
        {
            Ok(result) => {
                match serde_json::from_str::<NvimExecuteLuaResult<String>>(result.as_str().unwrap())
                {
                    Ok(rv) => {
                        let output: Result<String, NeovimError> = rv.into();
                        if let Ok(ref content) = output {
                            debug!(context_id = context_id, "Read document: {} bytes", content.len());
                        }
                        output
                    }
                    Err(e) => {
                        debug!("Failed to parse read document result: {}", e);
                        Err(NeovimError::Api(format!(
                            "Failed to parse read document result: {e}"
                        )))
                    }
                }
            }
            Err(e) => {
                debug!("Failed to get read document: {}", e);
                Err(NeovimError::Api(format!(
                    "Failed to get read document: {e}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_notification_tracker_basic() {
        let tracker = NotificationTracker::default();

        // Test recording a notification
        tracker
            .record_notification(
                "test_notification".to_string(),
                vec![Value::from("test_arg")],
            )
            .await;

        // Test waiting for the notification
        let result = tracker
            .wait_for_notification("test_notification", Duration::from_millis(100))
            .await;

        assert!(result.is_ok());
        let notification = result.unwrap();
        assert_eq!(notification.name, "test_notification");
        assert_eq!(notification.args.len(), 1);
        assert_eq!(notification.args[0].as_str().unwrap(), "test_arg");
    }

    #[tokio::test]
    async fn test_notification_tracker_timeout() {
        let tracker = NotificationTracker::default();

        // Test waiting for a notification that never comes (should timeout)
        let result = tracker
            .wait_for_notification("nonexistent_notification", Duration::from_millis(50))
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(matches!(error, NeovimError::Api(_)));
        assert!(
            error
                .to_string()
                .contains("Timeout waiting for notification")
        );
    }

    #[tokio::test]
    async fn test_notification_tracker_wait_then_send() {
        let tracker = NotificationTracker::default();

        // Spawn a task that will wait for a notification
        let wait_handle = tokio::spawn({
            let tracker = tracker.clone();
            async move {
                tracker
                    .wait_for_notification("test_async_notification", Duration::from_millis(500))
                    .await
            }
        });

        // Give the waiting task a moment to start waiting
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Now send the notification
        tracker
            .record_notification(
                "test_async_notification".to_string(),
                vec![Value::from("async_test_arg")],
            )
            .await;

        // The waiting task should now receive the notification
        let result = wait_handle.await.unwrap();
        assert!(result.is_ok());
        let notification = result.unwrap();
        assert_eq!(notification.name, "test_async_notification");
        assert_eq!(notification.args.len(), 1);
        assert_eq!(notification.args[0].as_str().unwrap(), "async_test_arg");
    }

    #[tokio::test]
    async fn test_notification_cleanup_expired() {
        let tracker = NotificationTracker::default();

        // Record a notification with a modified timestamp (simulate old notification)
        let old_notification = Notification {
            name: "old_notification".to_string(),
            args: vec![Value::from("old_data")],
            timestamp: std::time::SystemTime::now()
                - Duration::from_secs(NOTIFICATION_EXPIRY_SECONDS + 1),
        };

        // Manually insert old notification to simulate existing data
        {
            let mut notifications = tracker.notifications.lock().await;
            notifications.push(old_notification);
        }

        // Record a fresh notification
        tracker
            .record_notification(
                "fresh_notification".to_string(),
                vec![Value::from("fresh_data")],
            )
            .await;

        // Check initial state
        let (count_before, _) = tracker.get_stats().await;
        assert_eq!(count_before, 2);

        // Trigger cleanup
        tracker.cleanup_expired_notifications().await;

        // Old notification should be removed, fresh one should remain
        let (count_after, _) = tracker.get_stats().await;
        assert_eq!(count_after, 1);

        // Verify the remaining notification is the fresh one
        let result = tracker
            .wait_for_notification("fresh_notification", Duration::from_millis(10))
            .await;
        assert!(result.is_ok());

        // Verify old notification is gone
        let result = tracker
            .wait_for_notification("old_notification", Duration::from_millis(10))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_notification_cleanup_excess() {
        let tracker = NotificationTracker::default();

        // Record more than MAX_STORED_NOTIFICATIONS
        for i in 0..(MAX_STORED_NOTIFICATIONS + 10) {
            tracker
                .record_notification(format!("notification_{}", i), vec![Value::from(i as i64)])
                .await;
        }

        // Get current count
        let (count, _) = tracker.get_stats().await;

        // Should be limited to MAX_STORED_NOTIFICATIONS due to automatic cleanup
        assert!(count <= MAX_STORED_NOTIFICATIONS);

        // The most recent notifications should still be available
        let result = tracker
            .wait_for_notification(
                &format!("notification_{}", MAX_STORED_NOTIFICATIONS + 9),
                Duration::from_millis(10),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_notification_expiry_in_wait() {
        let tracker = NotificationTracker::default();

        // Create an expired notification manually
        let expired_notification = Notification {
            name: "expired_test".to_string(),
            args: vec![Value::from("expired_data")],
            timestamp: std::time::SystemTime::now()
                - Duration::from_secs(NOTIFICATION_EXPIRY_SECONDS + 1),
        };

        // Manually insert expired notification
        {
            let mut notifications = tracker.notifications.lock().await;
            notifications.push(expired_notification);
        }

        // wait_for_notification should not return expired notification
        let result = tracker
            .wait_for_notification("expired_test", Duration::from_millis(50))
            .await;

        // Should timeout because expired notification is ignored
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Timeout waiting for notification")
        );
    }
}
