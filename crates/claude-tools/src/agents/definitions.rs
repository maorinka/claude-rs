pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub model: Option<String>,
}

pub fn builtin_agents() -> Vec<AgentDefinition> {
    vec![
        AgentDefinition { name: "general-purpose".into(), description: "General-purpose agent".into(), system_prompt: "You are a general-purpose agent.".into(), model: None },
        AgentDefinition { name: "Explore".into(), description: "Fast codebase explorer".into(), system_prompt: "You are a fast exploration agent.".into(), model: Some("haiku".into()) },
        AgentDefinition { name: "Plan".into(), description: "Architecture planner".into(), system_prompt: "You are a software architect.".into(), model: None },
        AgentDefinition { name: "code-reviewer".into(), description: "Code reviewer".into(), system_prompt: "You review code for quality.".into(), model: None },
    ]
}
