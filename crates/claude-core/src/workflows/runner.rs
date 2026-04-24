//! Workflow runner: load workflow files and execute steps in sequence.

use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use tokio::process::Command;
use tracing;
use super::types::*;

fn parse_frontmatter(raw: &str) -> (WorkflowFrontmatter, &str) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (WorkflowFrontmatter::default(), raw);
    }
    if let Some(end) = trimmed[3..].find("\n---") {
        let yaml_block = &trimmed[3..3 + end];
        let rest = &trimmed[3 + end + 4..];
        match serde_json::from_str::<WorkflowFrontmatter>(&serde_yaml_front(yaml_block)) {
            Ok(fm) => (fm, rest),
            Err(_) => (WorkflowFrontmatter::default(), raw),
        }
    } else {
        (WorkflowFrontmatter::default(), raw)
    }
}

fn serde_yaml_front(yaml: &str) -> String {
    let mut obj = serde_json::Map::new();
    let mut current_key: Option<String> = None;
    let mut list_values: Vec<String> = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
        if let Some(item) = trimmed.strip_prefix("- ") {
            list_values.push(item.trim().to_string());
            continue;
        }
        if let Some(ref key) = current_key {
            if !list_values.is_empty() {
                let arr: Vec<serde_json::Value> = list_values.drain(..).map(serde_json::Value::String).collect();
                obj.insert(key.clone(), serde_json::Value::Array(arr));
                current_key = None;
            }
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim();
            if value.is_empty() {
                current_key = Some(key);
            } else {
                let v = value.trim_matches('"').trim_matches('\'');
                obj.insert(key, serde_json::Value::String(v.to_string()));
                current_key = None;
            }
        }
    }
    if let Some(ref key) = current_key {
        if !list_values.is_empty() {
            let arr: Vec<serde_json::Value> = list_values.drain(..).map(serde_json::Value::String).collect();
            obj.insert(key.clone(), serde_json::Value::Array(arr));
        }
    }
    serde_json::to_string(&obj).unwrap_or_else(|_| "{}".to_string())
}

fn parse_steps(body: &str) -> Vec<WorkflowStep> {
    let mut steps = Vec::new();
    let mut in_code = false;
    let mut code_buf = String::new();
    let mut prompt_buf = String::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code {
                let cmd = code_buf.trim().to_string();
                if !cmd.is_empty() {
                    steps.push(WorkflowStep::Command { command: cmd, cwd: None, description: None });
                }
                code_buf.clear();
                in_code = false;
            } else {
                flush_prompt(&mut prompt_buf, &mut steps);
                in_code = true;
            }
            continue;
        }
        if in_code {
            if !code_buf.is_empty() { code_buf.push('\n'); }
            code_buf.push_str(line);
        } else if !trimmed.is_empty() {
            if !prompt_buf.is_empty() { prompt_buf.push('\n'); }
            prompt_buf.push_str(trimmed);
        } else {
            flush_prompt(&mut prompt_buf, &mut steps);
        }
    }
    flush_prompt(&mut prompt_buf, &mut steps);
    if !code_buf.trim().is_empty() {
        steps.push(WorkflowStep::Command { command: code_buf.trim().to_string(), cwd: None, description: None });
    }
    steps
}

fn flush_prompt(buf: &mut String, steps: &mut Vec<WorkflowStep>) {
    let text = buf.trim().to_string();
    if !text.is_empty() { steps.push(WorkflowStep::Prompt { content: text, description: None }); }
    buf.clear();
}

fn slug_from_path(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown").to_string()
}

pub fn load_workflow(path: &Path, source: WorkflowSource) -> Result<WorkflowConfig> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (frontmatter, body) = parse_frontmatter(&raw);
    let steps = parse_steps(body);
    Ok(WorkflowConfig { slug: slug_from_path(path), source_path: path.to_path_buf(), source, frontmatter, steps })
}

pub fn load_workflows_from_dir(dir: &Path, source: WorkflowSource) -> Result<Vec<WorkflowConfig>> {
    if !dir.is_dir() { return Ok(Vec::new()); }
    let mut workflows = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            match load_workflow(&path, source) {
                Ok(wf) => workflows.push(wf),
                Err(e) => { tracing::warn!("skipping workflow {}: {e:#}", path.display()); }
            }
        }
    }
    workflows.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(workflows)
}

pub fn discover_workflows(project_root: &Path) -> Result<Vec<WorkflowConfig>> {
    let mut all = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let user_dir = home.join(".claude").join("workflows");
        all.extend(load_workflows_from_dir(&user_dir, WorkflowSource::User)?);
    }
    let project_dir = project_root.join(".claude").join("workflows");
    all.extend(load_workflows_from_dir(&project_dir, WorkflowSource::Project)?);
    Ok(all)
}

async fn run_step(step: &WorkflowStep, default_cwd: &Path) -> StepResult {
    match step {
        WorkflowStep::Prompt { content, .. } => StepResult { index: 0, success: true, output: content.clone() },
        WorkflowStep::Command { command, cwd, .. } => {
            let work_dir = cwd.as_ref().map(PathBuf::from).unwrap_or_else(|| default_cwd.to_path_buf());
            match Command::new("sh").arg("-c").arg(command).current_dir(&work_dir).output().await {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    let combined = if stderr.is_empty() { stdout } else { format!("{stdout}\n{stderr}") };
                    StepResult { index: 0, success: output.status.success(), output: combined }
                }
                Err(e) => StepResult { index: 0, success: false, output: format!("failed to spawn: {e}") },
            }
        }
    }
}

pub async fn run_workflow(workflow: &WorkflowConfig, cwd: &Path) -> WorkflowResult {
    let mut step_results = Vec::new();
    let mut all_ok = true;
    for (i, step) in workflow.steps.iter().enumerate() {
        let mut result = run_step(step, cwd).await;
        result.index = i;
        let ok = result.success;
        step_results.push(result);
        if !ok { all_ok = false; break; }
    }
    WorkflowResult { slug: workflow.slug.clone(), step_results, success: all_ok }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_steps_mixed() {
        let body = "Explain the codebase.\n\n```\nls -la\n```\n\nSummarise.\n";
        let steps = parse_steps(body);
        assert_eq!(steps.len(), 3);
        assert!(matches!(&steps[0], WorkflowStep::Prompt { content, .. } if content.contains("Explain")));
        assert!(matches!(&steps[1], WorkflowStep::Command { command, .. } if command == "ls -la"));
        assert!(matches!(&steps[2], WorkflowStep::Prompt { content, .. } if content.contains("Summarise")));
    }

    #[test]
    fn test_parse_frontmatter() {
        let raw = "---\nname: Deploy\ndescription: Deploy to prod\n---\nRun deploy.\n";
        let (fm, body) = parse_frontmatter(raw);
        assert_eq!(fm.name.as_deref(), Some("Deploy"));
        assert_eq!(fm.description.as_deref(), Some("Deploy to prod"));
        assert!(body.contains("Run deploy"));
    }

    #[test]
    fn test_load_workflow_from_file() {
        let tmp = TempDir::new().unwrap();
        let wf_path = tmp.path().join("deploy.md");
        fs::write(&wf_path, "---\nname: Deploy\n---\nCheck\n\n```\ngit status\n```\n\nConfirm.\n").unwrap();
        let wf = load_workflow(&wf_path, WorkflowSource::Project).unwrap();
        assert_eq!(wf.slug, "deploy");
        assert_eq!(wf.frontmatter.name.as_deref(), Some("Deploy"));
        assert_eq!(wf.steps.len(), 3);
    }

    #[test]
    fn test_discover_workflows_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(discover_workflows(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn test_load_workflows_from_dir() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path().join(".claude").join("workflows");
        fs::create_dir_all(&wf_dir).unwrap();
        fs::write(wf_dir.join("alpha.md"), "Do alpha\n").unwrap();
        fs::write(wf_dir.join("beta.md"), "Do beta\n").unwrap();
        fs::write(wf_dir.join("not-md.txt"), "ignored\n").unwrap();
        let workflows = load_workflows_from_dir(&wf_dir, WorkflowSource::Project).unwrap();
        assert_eq!(workflows.len(), 2);
        assert_eq!(workflows[0].slug, "alpha");
        assert_eq!(workflows[1].slug, "beta");
    }

    #[tokio::test]
    async fn test_run_workflow_command() {
        let tmp = TempDir::new().unwrap();
        let wf = WorkflowConfig {
            slug: "test".into(), source_path: tmp.path().join("test.md"),
            source: WorkflowSource::User, frontmatter: WorkflowFrontmatter::default(),
            steps: vec![WorkflowStep::Command { command: "echo hello".into(), cwd: None, description: None }],
        };
        let result = run_workflow(&wf, tmp.path()).await;
        assert!(result.success);
        assert!(result.step_results[0].output.contains("hello"));
    }

    #[tokio::test]
    async fn test_run_workflow_stops_on_failure() {
        let tmp = TempDir::new().unwrap();
        let wf = WorkflowConfig {
            slug: "fail".into(), source_path: tmp.path().join("fail.md"),
            source: WorkflowSource::User, frontmatter: WorkflowFrontmatter::default(),
            steps: vec![
                WorkflowStep::Command { command: "false".into(), cwd: None, description: None },
                WorkflowStep::Command { command: "echo nope".into(), cwd: None, description: None },
            ],
        };
        let result = run_workflow(&wf, tmp.path()).await;
        assert!(!result.success);
        assert_eq!(result.step_results.len(), 1);
    }

    #[test]
    fn test_frontmatter_allowed_tools() {
        let raw = "---\nname: R\nallowed_tools:\n  - Bash\n  - Read\n---\nDo.\n";
        let (fm, _) = parse_frontmatter(raw);
        let tools = fm.allowed_tools.as_ref().unwrap();
        assert_eq!(tools, &["Bash", "Read"]);
    }
}
