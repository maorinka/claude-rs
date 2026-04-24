use serde::{Deserialize, Serialize};

/// Sandbox configuration settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSettings {
    pub enabled: Option<bool>,
}
