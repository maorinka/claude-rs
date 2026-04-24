// Shared env-mutation lock for constants tests that set/remove process
// env vars. Different tests within this crate touch the same variables
// (USER_TYPE, ENABLE_GROWTHBOOK_DEV, CLAUDE_CODE_* overrides), so
// serialising them all with one lock prevents the race where one test's
// `remove_var` clobbers another's setup.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub mod api_limits;
pub mod betas;
pub mod common;
pub mod cyber_risk;
pub mod display;
pub mod files;
pub mod github_app;
pub mod keys;
pub mod messages;
pub mod oauth;
pub mod product;
pub mod system_prompt_sections;
pub mod tool_limits;
