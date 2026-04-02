use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

const MARKETPLACE_URL: &str = "https://plugins.claude.ai/api/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketplacePlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub download_url: String,
    pub install_count: u64,
}

pub struct MarketplaceClient {
    http: reqwest::Client,
}

impl Default for MarketplaceClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketplaceClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<MarketplacePlugin>> {
        let resp = self
            .http
            .get(format!("{}/search", MARKETPLACE_URL))
            .query(&[("q", query)])
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Marketplace search failed: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    pub async fn install(&self, plugin_id: &str, install_dir: &Path) -> Result<()> {
        let plugins = self.search(plugin_id).await?;
        let plugin = plugins
            .first()
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", plugin_id))?;

        let resp = self.http.get(&plugin.download_url).send().await?;
        let bytes = resp.bytes().await?;

        let plugin_dir = install_dir.join(&plugin.name);
        std::fs::create_dir_all(&plugin_dir)?;
        std::fs::write(plugin_dir.join("plugin.json"), &bytes)?;

        Ok(())
    }

    pub async fn list_installed(install_dir: &Path) -> Result<Vec<String>> {
        let mut installed = Vec::new();
        if install_dir.exists() {
            for entry in std::fs::read_dir(install_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    installed.push(entry.file_name().to_string_lossy().to_string());
                }
            }
        }
        Ok(installed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_marketplace_plugin_deserialization() {
        let json = r#"{
            "id": "plugin-001",
            "name": "my-plugin",
            "version": "1.0.0",
            "description": "A useful plugin",
            "author": "someone",
            "download_url": "https://example.com/download/my-plugin.tar.gz",
            "install_count": 42
        }"#;
        let plugin: MarketplacePlugin = serde_json::from_str(json).unwrap();
        assert_eq!(plugin.id, "plugin-001");
        assert_eq!(plugin.name, "my-plugin");
        assert_eq!(plugin.version, "1.0.0");
        assert_eq!(plugin.description, "A useful plugin");
        assert_eq!(plugin.author, "someone");
        assert_eq!(plugin.install_count, 42);
    }

    #[test]
    fn test_marketplace_plugin_serialization_roundtrip() {
        let plugin = MarketplacePlugin {
            id: "plugin-xyz".to_string(),
            name: "test-plugin".to_string(),
            version: "2.1.0".to_string(),
            description: "Test".to_string(),
            author: "tester".to_string(),
            download_url: "https://example.com/dl/test.zip".to_string(),
            install_count: 1000,
        };
        let json = serde_json::to_string(&plugin).unwrap();
        let deserialized: MarketplacePlugin = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, plugin.id);
        assert_eq!(deserialized.name, plugin.name);
        assert_eq!(deserialized.version, plugin.version);
        assert_eq!(deserialized.install_count, plugin.install_count);
    }

    #[tokio::test]
    async fn test_list_installed_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let result = MarketplaceClient::list_installed(tmp.path()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_list_installed_with_plugins() {
        let tmp = TempDir::new().unwrap();
        // Create fake plugin directories
        std::fs::create_dir(tmp.path().join("plugin-a")).unwrap();
        std::fs::create_dir(tmp.path().join("plugin-b")).unwrap();
        // Also create a file (should not appear in list)
        std::fs::write(tmp.path().join("not-a-dir.txt"), "data").unwrap();

        let result = MarketplaceClient::list_installed(tmp.path()).await.unwrap();
        assert_eq!(result.len(), 2);
        let mut sorted = result.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["plugin-a", "plugin-b"]);
    }

    #[tokio::test]
    async fn test_list_installed_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");
        // Should return empty vec, not an error
        let result = MarketplaceClient::list_installed(&nonexistent)
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_marketplace_client_new() {
        let _client = MarketplaceClient::new();
    }
}
