use std::collections::HashMap;
use anyhow::Result;

pub enum CommandType {
    Prompt,  // Returns text injected as user message
    Action,  // Side effects, no message injection
}

pub struct Command {
    pub name: String,
    pub description: String,
    pub command_type: CommandType,
    pub handler: Box<dyn CommandHandler>,
}

pub trait CommandHandler: Send + Sync {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult>;
}

pub struct CommandContext {
    pub working_directory: std::path::PathBuf,
    pub model: String,
}

pub enum CommandResult {
    Message(String), // Inject as user message
    Action(String),  // Print output, no message
    Error(String),   // Error message
}

pub struct CommandRegistry {
    commands: HashMap<String, Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    pub fn register(&mut self, cmd: Command) {
        self.commands.insert(cmd.name.clone(), cmd);
    }

    pub fn get(&self, name: &str) -> Option<&Command> {
        self.commands.get(name)
    }

    pub fn all(&self) -> Vec<&Command> {
        self.commands.values().collect()
    }

    pub fn search(&self, query: &str) -> Vec<&Command> {
        self.commands
            .values()
            .filter(|c| {
                c.name.contains(query)
                    || c.description
                        .to_lowercase()
                        .contains(&query.to_lowercase())
            })
            .collect()
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}
