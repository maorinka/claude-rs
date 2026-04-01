use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteTaskConfig {
    pub prompt: String,
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteTaskStatus {
    pub task_id: String,
    pub status: RemoteStatus,
    pub output: Option<String>,
    pub session_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteEnvironment {
    pub id: String,
    pub machine_name: String,
    pub directory: String,
    pub branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_task_config_construction() {
        let cfg = RemoteTaskConfig {
            prompt: "Fix the bug".to_string(),
            model: Some("claude-opus-4-5".to_string()),
            max_turns: Some(10),
            working_directory: Some("/tmp/project".to_string()),
        };
        assert_eq!(cfg.prompt, "Fix the bug");
        assert_eq!(cfg.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(cfg.max_turns, Some(10));
        assert_eq!(cfg.working_directory.as_deref(), Some("/tmp/project"));
    }

    #[test]
    fn test_remote_task_config_optional_fields() {
        let cfg = RemoteTaskConfig {
            prompt: "Do something".to_string(),
            model: None,
            max_turns: None,
            working_directory: None,
        };
        assert!(cfg.model.is_none());
        assert!(cfg.max_turns.is_none());
        assert!(cfg.working_directory.is_none());
    }

    #[test]
    fn test_remote_task_config_serialization() {
        let cfg = RemoteTaskConfig {
            prompt: "test prompt".to_string(),
            model: Some("claude-3-5-sonnet".to_string()),
            max_turns: Some(5),
            working_directory: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: RemoteTaskConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.prompt, cfg.prompt);
        assert_eq!(deserialized.model, cfg.model);
        assert_eq!(deserialized.max_turns, cfg.max_turns);
    }

    #[test]
    fn test_remote_status_variants_serialization() {
        let cases = vec![
            (RemoteStatus::Pending, "pending"),
            (RemoteStatus::Running, "running"),
            (RemoteStatus::Completed, "completed"),
            (RemoteStatus::Failed, "failed"),
            (RemoteStatus::Cancelled, "cancelled"),
        ];
        for (status, expected_str) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{}\"", expected_str));
            let deserialized: RemoteStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }

    #[test]
    fn test_remote_task_status_serialization() {
        let status = RemoteTaskStatus {
            task_id: "task-123".to_string(),
            status: RemoteStatus::Running,
            output: Some("partial output".to_string()),
            session_url: Some("https://session.example.com/abc".to_string()),
        };
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: RemoteTaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.task_id, status.task_id);
        assert_eq!(deserialized.status, status.status);
        assert_eq!(deserialized.output, status.output);
        assert_eq!(deserialized.session_url, status.session_url);
    }

    #[test]
    fn test_remote_environment_serialization() {
        let env = RemoteEnvironment {
            id: "env-abc".to_string(),
            machine_name: "prod-machine-01".to_string(),
            directory: "/home/user/project".to_string(),
            branch: Some("main".to_string()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let deserialized: RemoteEnvironment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, env.id);
        assert_eq!(deserialized.machine_name, env.machine_name);
        assert_eq!(deserialized.directory, env.directory);
        assert_eq!(deserialized.branch, env.branch);
    }
}
