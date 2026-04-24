use super::types::*;
use anyhow::Result;

const REMOTE_API_URL: &str = "https://api.anthropic.com/v1/environments";

pub struct RemoteClient {
    http: reqwest::Client,
    auth_token: String,
}

impl RemoteClient {
    pub fn new(auth_token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            auth_token: auth_token.to_string(),
        }
    }

    /// POST to remote API to create a cloud execution task.
    /// This requires Anthropic's cloud infrastructure.
    pub async fn create_task(&self, config: RemoteTaskConfig) -> Result<RemoteTaskStatus> {
        let resp = self
            .http
            .post(format!("{}/tasks", REMOTE_API_URL))
            .bearer_auth(&self.auth_token)
            .json(&config)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Remote task creation failed: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    pub async fn get_task_status(&self, task_id: &str) -> Result<RemoteTaskStatus> {
        let resp = self
            .http
            .get(format!("{}/tasks/{}", REMOTE_API_URL, task_id))
            .bearer_auth(&self.auth_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to get task status: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/tasks/{}/cancel", REMOTE_API_URL, task_id))
            .bearer_auth(&self.auth_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to cancel task: {}", resp.status());
        }
        Ok(())
    }

    pub async fn register_environment(&self, env: &RemoteEnvironment) -> Result<String> {
        let resp = self
            .http
            .post(format!("{}/bridge", REMOTE_API_URL))
            .bearer_auth(&self.auth_token)
            .json(env)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to register environment: {}", resp.status());
        }
        let data: serde_json::Value = resp.json().await?;
        Ok(data["environment_id"].as_str().unwrap_or("").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_client_construction() {
        let client = RemoteClient::new("test-token-abc");
        assert_eq!(client.auth_token, "test-token-abc");
    }

    #[test]
    fn test_remote_client_empty_token() {
        let client = RemoteClient::new("");
        assert!(client.auth_token.is_empty());
    }
}
