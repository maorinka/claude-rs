use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::client::LspClient;
use super::types::Diagnostic;

/// Configuration for a registered LSP server.
#[derive(Debug, Clone)]
pub struct LspServerConfig {
    /// The language ID this server handles (e.g. "rust", "typescript", "python").
    pub language_id: String,
    /// Command to spawn the server process.
    pub command: String,
    /// Arguments passed to the server command.
    pub args: Vec<String>,
    /// File extensions this server handles (e.g. [".rs"], [".ts", ".tsx"]).
    pub extensions: Vec<String>,
}

/// Manages multiple LSP servers, routing requests to the appropriate server
/// based on file type (extension).
///
/// This mirrors the TS LSPServerManager which manages server instances by
/// language ID and routes file operations to the correct server.
pub struct LspManager {
    /// Registered server configurations, keyed by language_id.
    configs: HashMap<String, LspServerConfig>,
    /// Running LSP clients, keyed by language_id. Wrapped in RwLock for
    /// concurrent access (reads during diagnostics, writes during start/stop).
    clients: Arc<RwLock<HashMap<String, LspClient>>>,
    /// Map from file extension (e.g. ".rs") to language_id.
    extension_map: HashMap<String, String>,
    /// The workspace root URI used for initialization.
    root_uri: Option<String>,
}

impl LspManager {
    /// Create a new LSP manager with no servers registered.
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            clients: Arc::new(RwLock::new(HashMap::new())),
            extension_map: HashMap::new(),
            root_uri: None,
        }
    }

    /// Set the workspace root URI for server initialization.
    pub fn set_root_uri(&mut self, root_uri: String) {
        self.root_uri = Some(root_uri);
    }

    /// Register a language server configuration.
    ///
    /// The server will be lazily started on first use (when a file with a
    /// matching extension is opened or diagnostics are requested).
    pub fn register_server(
        &mut self,
        language_id: &str,
        command: &str,
        args: &[String],
        extensions: &[String],
    ) {
        debug!(
            language_id = language_id,
            command = command,
            "Registering LSP server"
        );

        for ext in extensions {
            self.extension_map
                .insert(ext.clone(), language_id.to_string());
        }

        self.configs.insert(
            language_id.to_string(),
            LspServerConfig {
                language_id: language_id.to_string(),
                command: command.to_string(),
                args: args.to_vec(),
                extensions: extensions.to_vec(),
            },
        );
    }

    /// Get the language ID for a file path based on its extension.
    pub fn language_id_for_path(&self, file_path: &str) -> Option<&str> {
        let path = Path::new(file_path);
        let ext = path.extension()?.to_str()?;
        let ext_with_dot = format!(".{}", ext);
        self.extension_map.get(&ext_with_dot).map(|s| s.as_str())
    }

    /// Ensure the server for the given language ID is started.
    /// Returns an error if no server is registered for this language or if
    /// the server fails to start.
    async fn ensure_server_started(&self, language_id: &str) -> Result<()> {
        // Check if already running
        {
            let clients = self.clients.read().await;
            if clients.contains_key(language_id) {
                return Ok(());
            }
        }

        // Look up config
        let config = self
            .configs
            .get(language_id)
            .ok_or_else(|| anyhow!("No LSP server registered for language: {}", language_id))?
            .clone();

        // Start the server
        let mut client =
            LspClient::start(&config.language_id, &config.command, &config.args).await?;

        // Initialize with workspace root
        let root_uri = self.root_uri.clone().unwrap_or_else(|| {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            format!("file://{}", cwd.display())
        });

        client.initialize(&root_uri).await?;

        info!(
            language_id = language_id,
            command = config.command,
            "LSP server started and initialized"
        );

        let mut clients = self.clients.write().await;
        clients.insert(language_id.to_string(), client);
        Ok(())
    }

    /// Get diagnostics for a file.
    ///
    /// Finds the appropriate LSP server based on file extension, starts it if
    /// needed, and requests diagnostics. Returns an empty vec if no server
    /// handles this file type.
    pub async fn get_diagnostics(&self, file_path: &str) -> Result<Vec<Diagnostic>> {
        let language_id = match self.language_id_for_path(file_path) {
            Some(id) => id.to_string(),
            None => {
                debug!(
                    file_path = file_path,
                    "No LSP server registered for this file type"
                );
                return Ok(vec![]);
            }
        };

        // Ensure server is started
        if let Err(e) = self.ensure_server_started(&language_id).await {
            warn!(
                language_id = language_id,
                error = %e,
                "Failed to start LSP server for diagnostics"
            );
            return Ok(vec![]);
        }

        let uri = path_to_file_uri(file_path);
        let clients = self.clients.read().await;
        if let Some(client) = clients.get(&language_id) {
            client.text_document_diagnostics(&uri).await
        } else {
            Ok(vec![])
        }
    }

    /// Notify the appropriate LSP server that a file has changed.
    ///
    /// Opens the file in the server if not already open, or sends a change
    /// notification. Determines the correct server from the file extension.
    pub async fn notify_change(&self, file_path: &str, content: &str) -> Result<()> {
        let language_id = match self.language_id_for_path(file_path) {
            Some(id) => id.to_string(),
            None => {
                debug!(
                    file_path = file_path,
                    "No LSP server registered for this file type"
                );
                return Ok(());
            }
        };

        // Ensure server is started
        self.ensure_server_started(&language_id).await?;

        let uri = path_to_file_uri(file_path);
        let mut clients = self.clients.write().await;
        if let Some(client) = clients.get_mut(&language_id) {
            // Send didOpen for the first notification, didChange for subsequent ones.
            // We always send didChange here since we use full document sync and
            // the server should handle receiving didChange without a prior didOpen
            // gracefully (or we can track open state more precisely in the future).
            client.text_document_did_change(&uri, content).await
        } else {
            Ok(())
        }
    }

    /// Open a file in the appropriate LSP server.
    ///
    /// Sends textDocument/didOpen with the file content.
    pub async fn open_file(&self, file_path: &str, content: &str) -> Result<()> {
        let language_id = match self.language_id_for_path(file_path) {
            Some(id) => id.to_string(),
            None => return Ok(()),
        };

        self.ensure_server_started(&language_id).await?;

        let uri = path_to_file_uri(file_path);
        let mut clients = self.clients.write().await;
        if let Some(client) = clients.get_mut(&language_id) {
            client
                .text_document_did_open(&uri, &language_id, content)
                .await
        } else {
            Ok(())
        }
    }

    /// Send an arbitrary LSP request for a file.
    ///
    /// Routes the request to the appropriate server based on file extension.
    /// Returns None if no server handles this file type.
    pub async fn send_request(
        &self,
        file_path: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        let language_id = match self.language_id_for_path(file_path) {
            Some(id) => id.to_string(),
            None => return Ok(None),
        };

        self.ensure_server_started(&language_id).await?;

        let clients = self.clients.read().await;
        if let Some(client) = clients.get(&language_id) {
            client.send_lsp_request(method, Some(params)).await
        } else {
            Ok(None)
        }
    }

    /// Shut down all running LSP servers.
    pub async fn shutdown(&self) {
        let mut clients = self.clients.write().await;
        let language_ids: Vec<String> = clients.keys().cloned().collect();

        for language_id in language_ids {
            if let Some(mut client) = clients.remove(&language_id) {
                if let Err(e) = client.shutdown().await {
                    warn!(
                        language_id = language_id,
                        error = %e,
                        "Error shutting down LSP server"
                    );
                }
            }
        }

        info!("All LSP servers shut down");
    }

    /// Check if any LSP servers are currently running.
    pub async fn has_servers(&self) -> bool {
        !self.clients.read().await.is_empty()
    }

    /// Get the number of registered server configurations.
    pub fn registered_count(&self) -> usize {
        self.configs.len()
    }

    /// Get the names of all registered language IDs.
    pub fn registered_languages(&self) -> Vec<String> {
        self.configs.keys().cloned().collect()
    }
}

impl Default for LspManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a file path to a file:// URI.
fn path_to_file_uri(file_path: &str) -> String {
    // Handle absolute paths
    if file_path.starts_with('/') {
        format!("file://{}", file_path)
    } else {
        // Relative path - resolve against cwd
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let absolute = cwd.join(file_path);
        format!("file://{}", absolute.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_file_uri() {
        assert_eq!(
            path_to_file_uri("/home/user/project/src/main.rs"),
            "file:///home/user/project/src/main.rs"
        );
    }

    #[test]
    fn test_language_id_for_path() {
        let mut manager = LspManager::new();
        manager.register_server("rust", "rust-analyzer", &[], &[".rs".to_string()]);
        manager.register_server(
            "typescript",
            "typescript-language-server",
            &["--stdio".to_string()],
            &[".ts".to_string(), ".tsx".to_string()],
        );

        assert_eq!(
            manager.language_id_for_path("/project/src/main.rs"),
            Some("rust")
        );
        assert_eq!(
            manager.language_id_for_path("/project/src/app.ts"),
            Some("typescript")
        );
        assert_eq!(
            manager.language_id_for_path("/project/src/component.tsx"),
            Some("typescript")
        );
        assert_eq!(manager.language_id_for_path("/project/readme.md"), None);
    }

    #[test]
    fn test_register_server() {
        let mut manager = LspManager::new();
        assert_eq!(manager.registered_count(), 0);

        manager.register_server("python", "pylsp", &[], &[".py".to_string()]);
        assert_eq!(manager.registered_count(), 1);
        assert_eq!(manager.registered_languages(), vec!["python".to_string()]);
    }

    #[tokio::test]
    async fn test_empty_manager_get_diagnostics() {
        let manager = LspManager::new();
        let diagnostics = manager.get_diagnostics("/some/file.rs").await.unwrap();
        assert!(diagnostics.is_empty());
    }

    #[tokio::test]
    async fn test_empty_manager_notify_change() {
        let manager = LspManager::new();
        // Should succeed silently with no servers
        manager
            .notify_change("/some/file.rs", "fn main() {}")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_empty_manager_has_servers() {
        let manager = LspManager::new();
        assert!(!manager.has_servers().await);
    }

    #[tokio::test]
    async fn test_empty_manager_shutdown() {
        let manager = LspManager::new();
        // Should complete without error
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn test_unregistered_extension_returns_empty() {
        let mut manager = LspManager::new();
        manager.register_server("rust", "rust-analyzer", &[], &[".rs".to_string()]);

        // .py is not registered, should return empty
        let diagnostics = manager.get_diagnostics("/some/file.py").await.unwrap();
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_set_root_uri() {
        let mut manager = LspManager::new();
        assert!(manager.root_uri.is_none());

        manager.set_root_uri("file:///home/user/project".to_string());
        assert_eq!(
            manager.root_uri,
            Some("file:///home/user/project".to_string())
        );
    }

    #[tokio::test]
    async fn test_send_request_no_server() {
        let manager = LspManager::new();
        let result = manager
            .send_request("/some/file.rs", "textDocument/hover", serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }
}
