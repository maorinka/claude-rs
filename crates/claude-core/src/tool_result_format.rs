use serde_json::Value;

pub fn unknown_tool_error_text(tool_name: &str) -> String {
    format!("Error: No such tool available: {tool_name}")
}

pub fn unknown_tool_error_content(tool_name: &str) -> String {
    format!(
        "<tool_use_error>{}</tool_use_error>",
        unknown_tool_error_text(tool_name)
    )
}

pub fn format_tool_result_for_model(tool_name: &str, data: &Value) -> String {
    ensure_non_empty_tool_result_string(
        tool_name,
        format_tool_result_string_for_model(tool_name, data),
    )
}

pub fn format_tool_result_content_for_model(tool_name: &str, data: &Value) -> Value {
    if tool_name == "Agent" || tool_name == "agent" {
        return ensure_non_empty_tool_result_content(
            tool_name,
            format_agent_tool_result_content_for_model(data),
        );
    }

    if tool_name == "ToolSearch" {
        let matches = data
            .get("matches")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .or_else(|| {
                data.get("tools").and_then(Value::as_array).map(|items| {
                    items
                        .iter()
                        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        if matches.is_empty() {
            let mut text = "No matching deferred tools found".to_string();
            if let Some(pending) = data
                .get("pending_mcp_servers")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
            {
                let names = pending
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                text.push_str(&format!(". Some MCP servers are still connecting: {names}. Their tools will become available shortly — try searching again."));
            }
            return ensure_non_empty_tool_result_content(tool_name, Value::String(text));
        }
        return ensure_non_empty_tool_result_content(
            tool_name,
            Value::Array(
                matches
                    .into_iter()
                    .map(|name| serde_json::json!({"type": "tool_reference", "tool_name": name}))
                    .collect(),
            ),
        );
    }

    ensure_non_empty_tool_result_content(
        tool_name,
        Value::String(format_tool_result_string_for_model(tool_name, data)),
    )
}

pub fn ensure_non_empty_tool_result_string(tool_name: &str, content: String) -> String {
    if content.trim().is_empty() {
        format!("({tool_name} completed with no output)")
    } else {
        content
    }
}

pub fn ensure_non_empty_tool_result_content(tool_name: &str, content: Value) -> Value {
    if is_tool_result_content_empty(&content) {
        Value::String(format!("({tool_name} completed with no output)"))
    } else {
        content
    }
}

pub fn is_tool_result_content_empty(content: &Value) -> bool {
    match content {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(blocks) => {
            blocks.is_empty()
                || blocks.iter().all(|block| {
                    block.get("type").and_then(Value::as_str) == Some("text")
                        && block
                            .get("text")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .unwrap_or("")
                            .is_empty()
                })
        }
        _ => false,
    }
}

fn format_bash_tool_result_for_model(data: &Value) -> String {
    let Some(obj) = data.as_object() else {
        return data.as_str().unwrap_or(&data.to_string()).to_string();
    };
    let stdout = obj.get("stdout").and_then(Value::as_str).unwrap_or("");
    let stderr = obj.get("stderr").and_then(Value::as_str).unwrap_or("");
    let mut parts = Vec::new();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, true) => parts.push(stdout.trim_end_matches('\n').to_string()),
        (true, false) => parts.push(stderr.trim_end_matches('\n').to_string()),
        (false, false) => parts.push(
            format!("{stdout}\n{stderr}")
                .trim_end_matches('\n')
                .to_string(),
        ),
        (true, true) => {}
    }

    let task_id = obj
        .get("backgroundTaskId")
        .or_else(|| obj.get("task_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let output_path = obj
        .get("outputPath")
        .or_else(|| obj.get("output_file"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !task_id.is_empty() && !output_path.is_empty() {
        let background_info = if obj
            .get("assistantAutoBackgrounded")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            format!("Command exceeded the assistant-mode blocking budget (10s) and was moved to the background with ID: {task_id}. It is still running — you will be notified when it completes. Output is being written to: {output_path}. In assistant mode, delegate long-running work to a subagent or use run_in_background to keep this conversation responsive.")
        } else if obj
            .get("backgroundedByUser")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            format!("Command was manually backgrounded by user with ID: {task_id}. Output is being written to: {output_path}")
        } else {
            format!("Command running in background with ID: {task_id}. Output is being written to: {output_path}")
        };
        parts.push(background_info);
    }

    parts.retain(|part| !part.is_empty());
    parts.join("\n")
}

fn add_line_numbers_ts(content: &str, start_line: usize) -> String {
    if content.is_empty() {
        return String::new();
    }
    content
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .enumerate()
        .map(|(index, line)| format!("{}\t{}", start_line + index, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_single_line(text: &str, max_width: usize) -> String {
    let first_line = text.split('\n').next().unwrap_or("");
    let had_newline = first_line.len() != text.len();
    let mut truncated: String = first_line.chars().take(max_width).collect();
    let over_width = first_line.chars().count() > max_width;
    if over_width {
        if max_width <= 1 {
            return "...".to_string();
        }
        truncated = first_line.chars().take(max_width - 1).collect();
    }
    if had_newline || over_width {
        truncated.push_str("...");
    }
    truncated
}

fn format_agent_tool_result_content_for_model(data: &Value) -> Value {
    let Some(status) = data.get("status").and_then(Value::as_str) else {
        return Value::String(data.to_string());
    };

    if status == "teammate_spawned" {
        let teammate_id = data
            .get("teammate_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let name = data.get("name").and_then(Value::as_str).unwrap_or("");
        let team_name = data.get("team_name").and_then(Value::as_str).unwrap_or("");
        return serde_json::json!([{
            "type": "text",
            "text": format!("Spawned successfully.\nagent_id: {teammate_id}\nname: {name}\nteam_name: {team_name}\nThe agent is now running and will receive instructions via mailbox.")
        }]);
    }

    if status == "remote_launched" {
        let task_id = data.get("taskId").and_then(Value::as_str).unwrap_or("");
        let session_url = data.get("sessionUrl").and_then(Value::as_str).unwrap_or("");
        let output_file = data.get("outputFile").and_then(Value::as_str).unwrap_or("");
        return serde_json::json!([{
            "type": "text",
            "text": format!("Remote agent launched in CCR.\ntaskId: {task_id}\nsession_url: {session_url}\noutput_file: {output_file}\nThe agent is running remotely. You will be notified automatically when it completes.\nBriefly tell the user what you launched and end your response.")
        }]);
    }

    if status == "async_launched" {
        let agent_id = data.get("agentId").and_then(Value::as_str).unwrap_or("");
        let prefix = format!(
            "Async agent launched successfully.\nagentId: {agent_id} (internal ID - do not mention to user. Use SendMessage with to: '{agent_id}' to continue this agent.)\nThe agent is working in the background. You will be notified automatically when it completes."
        );
        let instructions = if data
            .get("canReadOutputFile")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let output_file = data.get("outputFile").and_then(Value::as_str).unwrap_or("");
            format!("Do not duplicate this agent's work — avoid working with the same files or topics it is using. Work on non-overlapping tasks, or briefly tell the user what you launched and end your response.\noutput_file: {output_file}\nIf asked, you can check progress before completion by using Read or Bash tail on the output file.")
        } else {
            "Briefly tell the user what you launched and end your response. Do not generate any other text — agent results will arrive in a subsequent message.".to_string()
        };
        return serde_json::json!([{ "type": "text", "text": format!("{prefix}\n{instructions}") }]);
    }

    if status == "completed" {
        let mut content = data
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if content.is_empty() {
            content.push(serde_json::json!({
                "type": "text",
                "text": "(Subagent completed but returned no output.)"
            }));
        }

        let worktree_info = match (
            data.get("worktreePath").and_then(Value::as_str),
            data.get("worktreeBranch").and_then(Value::as_str),
        ) {
            (Some(path), Some(branch)) => {
                format!("\nworktreePath: {path}\nworktreeBranch: {branch}")
            }
            _ => String::new(),
        };
        let one_shot = data
            .get("agentType")
            .and_then(Value::as_str)
            .map(|agent_type| matches!(agent_type, "Explore" | "Plan"))
            .unwrap_or(false);
        if one_shot && worktree_info.is_empty() {
            return Value::Array(content);
        }

        let agent_id = data.get("agentId").and_then(Value::as_str).unwrap_or("");
        let total_tokens = data.get("totalTokens").and_then(Value::as_i64).unwrap_or(0);
        let total_tool_use_count = data
            .get("totalToolUseCount")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let total_duration_ms = data
            .get("totalDurationMs")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        content.push(serde_json::json!({
            "type": "text",
            "text": format!("agentId: {agent_id} (use SendMessage with to: '{agent_id}' to continue this agent){worktree_info}\n<usage>total_tokens: {total_tokens}\ntool_uses: {total_tool_use_count}\nduration_ms: {total_duration_ms}</usage>")
        }));
        return Value::Array(content);
    }

    Value::String(data.to_string())
}

fn format_tool_result_string_for_model(tool_name: &str, data: &Value) -> String {
    if tool_name == "Bash" {
        return format_bash_tool_result_for_model(data);
    }
    if tool_name == "Monitor" {
        let task_id = data
            .get("backgroundTaskId")
            .or_else(|| data.get("task_id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let output_path = data
            .get("outputPath")
            .or_else(|| data.get("output_file"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !task_id.is_empty() && !output_path.is_empty() {
            return format!("Monitor running in background with ID: {task_id}. Output is being written to: {output_path}");
        }
        return data.to_string();
    }
    if tool_name == "AskUserQuestion" || tool_name == "AskUser" {
        let answers_text = data
            .get("answers")
            .and_then(Value::as_object)
            .map(|answers| {
                answers
                    .iter()
                    .map(|(question_text, answer)| {
                        let answer = answer.as_str().unwrap_or("");
                        let mut parts = vec![format!("\"{question_text}\"=\"{answer}\"")];
                        let annotation = data
                            .get("annotations")
                            .and_then(Value::as_object)
                            .and_then(|annotations| annotations.get(question_text));
                        if let Some(preview) = annotation
                            .and_then(|value| value.get("preview"))
                            .and_then(Value::as_str)
                        {
                            parts.push(format!("selected preview:\n{preview}"));
                        }
                        if let Some(notes) = annotation
                            .and_then(|value| value.get("notes"))
                            .and_then(Value::as_str)
                        {
                            parts.push(format!("user notes: {notes}"));
                        }
                        parts.join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        return format!("User has answered your questions: {answers_text}. You can now continue with the user's answers in mind.");
    }
    if tool_name == "LSP" {
        return data
            .get("result")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_else(|| data.to_string());
    }
    if tool_name == "SendMessage" || tool_name == "TaskStop" {
        return data.to_string();
    }
    if tool_name == "CronCreate" || tool_name == "ScheduleCron" {
        let id = data.get("id").and_then(Value::as_str).unwrap_or("");
        let human_schedule = data
            .get("humanSchedule")
            .and_then(Value::as_str)
            .unwrap_or("");
        let recurring = data
            .get("recurring")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let durable = data.get("durable").and_then(Value::as_bool).unwrap_or(true);
        let where_text = if durable {
            "Persisted to .claude/scheduled_tasks.json"
        } else {
            "Session-only (not written to disk, dies when Claude exits)"
        };
        if recurring {
            return format!(
                "Scheduled recurring job {id} ({human_schedule}). {where_text}. Auto-expires after 7 days. Use CronDelete to cancel sooner."
            );
        }
        return format!(
            "Scheduled one-shot task {id} ({human_schedule}). {where_text}. It will fire once then auto-delete."
        );
    }
    if tool_name == "CronDelete" {
        let id = data.get("id").and_then(Value::as_str).unwrap_or("");
        return format!("Cancelled job {id}.");
    }
    if tool_name == "CronList" {
        let Some(jobs) = data.get("jobs").and_then(Value::as_array) else {
            return "No scheduled jobs.".to_string();
        };
        if jobs.is_empty() {
            return "No scheduled jobs.".to_string();
        }
        return jobs
            .iter()
            .map(|job| {
                let id = job.get("id").and_then(Value::as_str).unwrap_or("");
                let human_schedule = job
                    .get("humanSchedule")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let recurring = job
                    .get("recurring")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let durable_suffix = if job.get("durable").and_then(Value::as_bool) == Some(false) {
                    " [session-only]"
                } else {
                    ""
                };
                let prompt = job.get("prompt").and_then(Value::as_str).unwrap_or("");
                format!(
                    "{id} — {human_schedule}{}{}: {}",
                    if recurring {
                        " (recurring)"
                    } else {
                        " (one-shot)"
                    },
                    durable_suffix,
                    truncate_single_line(prompt, 80)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if tool_name == "RemoteTrigger" {
        if let (Some(status), Some(json)) = (
            data.get("status").and_then(Value::as_i64),
            data.get("json").and_then(Value::as_str),
        ) {
            return format!("HTTP {status}\n{json}");
        }
        return data.to_string();
    }
    if tool_name == "EnterPlanMode" {
        let message = data
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.");
        let mut content = format!(
            "{message}\n\nIn plan mode, you should:\n1. Thoroughly explore the codebase to understand existing patterns\n2. Identify similar features and architectural approaches\n3. Consider multiple approaches and their trade-offs\n4. Use AskUserQuestion if you need to clarify the approach\n5. Design a concrete implementation strategy\n6. When ready, use ExitPlanMode to present your plan for approval\n\nRemember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase."
        );
        if let Some(instructions) = data.get("instructions").and_then(Value::as_str) {
            content.push_str("\n\n<system-reminder>\n");
            content.push_str(instructions);
            content.push_str("\n</system-reminder>");
        }
        return content;
    }
    if tool_name == "ExitPlanMode" {
        if data
            .get("isAgent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return "User has approved the plan. There is nothing else needed from you now. Please respond with \"ok\"".to_string();
        }
        let plan = data.get("plan").and_then(Value::as_str).unwrap_or("");
        if plan.trim().is_empty() {
            return "User has approved exiting plan mode. You can now proceed.".to_string();
        }
        let file_path = data.get("filePath").and_then(Value::as_str).unwrap_or("");
        let team_hint = if data
            .get("hasTaskTool")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "\n\nIf this plan can be broken down into multiple independent tasks, consider using the Task tool to create a team and parallelize the work."
        } else {
            ""
        };
        let plan_label = if data
            .get("planWasEdited")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "Approved Plan (edited by user)"
        } else {
            "Approved Plan"
        };
        return format!(
            "User has approved your plan. You can now start coding. Start with updating your todo list if applicable\n\nYour plan has been saved to: {file_path}\nYou can refer back to it if needed during implementation.{team_hint}\n\n## {plan_label}:\n{plan}"
        );
    }
    if tool_name == "Write" {
        if let (Some(file_path), Some(write_type)) = (
            data.get("filePath").and_then(Value::as_str),
            data.get("type").and_then(Value::as_str),
        ) {
            return match write_type {
                "create" => format!("File created successfully at: {file_path}"),
                "update" => format!("The file {file_path} has been updated successfully."),
                _ => data.to_string(),
            };
        }
    }
    if tool_name == "Edit" {
        if let Some(file_path) = data.get("filePath").and_then(Value::as_str) {
            let modified_note = if data
                .get("userModified")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                ".  The user modified your proposed changes before accepting them. "
            } else {
                ""
            };
            if data
                .get("replaceAll")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return format!("The file {file_path} has been updated{modified_note}. All occurrences were successfully replaced.");
            }
            return format!("The file {file_path} has been updated successfully{modified_note}.");
        }
    }
    if tool_name == "TodoWrite" {
        let base = "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable";
        if data
            .get("verificationNudgeNeeded")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return format!("{base}\n\nNOTE: You just closed out 3+ tasks and none of them was a verification step. Before writing your final summary, spawn the verification agent (subagent_type=\"verification\"). You cannot self-assign PARTIAL by listing caveats in your summary — only the verifier issues a verdict.");
        }
        return base.to_string();
    }
    if tool_name == "NotebookEdit" {
        if let Some(error) = data.get("error").and_then(Value::as_str) {
            if !error.is_empty() {
                return error.to_string();
            }
        }
        let cell_id = data.get("cell_id").and_then(Value::as_str).unwrap_or("");
        let new_source = data.get("new_source").and_then(Value::as_str).unwrap_or("");
        return match data.get("edit_mode").and_then(Value::as_str) {
            Some("replace") => format!("Updated cell {cell_id} with {new_source}"),
            Some("insert") => format!("Inserted cell {cell_id} with {new_source}"),
            Some("delete") => format!("Deleted cell {cell_id}"),
            _ => "Unknown edit mode".to_string(),
        };
    }
    if tool_name == "Skill" {
        if let Some(status) = data.get("status").and_then(Value::as_str) {
            if status == "forked" {
                let command_name = data
                    .get("commandName")
                    .or_else(|| data.get("skill"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let result = data.get("result").and_then(Value::as_str).unwrap_or("");
                return format!(
                    "Skill \"{command_name}\" completed (forked execution).\n\nResult:\n{result}"
                );
            }
        }
        let command_name = data
            .get("commandName")
            .or_else(|| data.get("skill"))
            .and_then(Value::as_str)
            .unwrap_or("");
        return format!("Launching skill: {command_name}");
    }
    if tool_name == "TaskOutput" {
        let mut parts = vec![format!(
            "<retrieval_status>{}</retrieval_status>",
            data.get("retrieval_status")
                .and_then(Value::as_str)
                .unwrap_or("found")
        )];
        let task_value = data.get("task").unwrap_or(data);
        if let Some(task_id) = task_value
            .get("task_id")
            .or_else(|| task_value.get("taskId"))
            .and_then(Value::as_str)
        {
            parts.push(format!("<task_id>{task_id}</task_id>"));
            let task_type = task_value
                .get("task_type")
                .or_else(|| task_value.get("taskType"))
                .and_then(Value::as_str)
                .unwrap_or("background");
            parts.push(format!("<task_type>{task_type}</task_type>"));
            if let Some(status) = task_value.get("status").and_then(Value::as_str) {
                parts.push(format!("<status>{status}</status>"));
            }
            if let Some(exit_code) = task_value
                .get("exitCode")
                .or_else(|| task_value.get("exit_code"))
                .and_then(Value::as_i64)
            {
                parts.push(format!("<exit_code>{exit_code}</exit_code>"));
            }
            if let Some(output) = task_value.get("output").and_then(Value::as_str) {
                if !output.trim().is_empty() {
                    parts.push(format!("<output>\n{}\n</output>", output.trim_end()));
                }
            }
            if let Some(error) = task_value.get("error").and_then(Value::as_str) {
                parts.push(format!("<error>{error}</error>"));
            }
        }
        return parts.join("\n\n");
    }
    if tool_name == "TaskCreate" {
        let task = data.get("task").unwrap_or(data);
        if let (Some(id), Some(subject)) = (
            task.get("id").and_then(Value::as_str),
            task.get("subject").and_then(Value::as_str),
        ) {
            return format!("Task #{id} created successfully: {subject}");
        }
    }
    if tool_name == "TaskGet" {
        let task = data.get("task").unwrap_or(data);
        if task.is_null() {
            return "Task not found".to_string();
        }
        if let Some(id) = task.get("id").and_then(Value::as_str) {
            let subject = task.get("subject").and_then(Value::as_str).unwrap_or("");
            let status = task.get("status").and_then(Value::as_str).unwrap_or("");
            let description = task
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut lines = vec![
                format!("Task #{id}: {subject}"),
                format!("Status: {status}"),
                format!("Description: {description}"),
            ];
            if let Some(blocked_by) = task.get("blockedBy").and_then(Value::as_array) {
                if !blocked_by.is_empty() {
                    lines.push(format!(
                        "Blocked by: {}",
                        blocked_by
                            .iter()
                            .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if let Some(blocks) = task.get("blocks").and_then(Value::as_array) {
                if !blocks.is_empty() {
                    lines.push(format!(
                        "Blocks: {}",
                        blocks
                            .iter()
                            .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            return lines.join("\n");
        }
    }
    if tool_name == "TaskList" {
        if let Some(tasks) = data.get("tasks").and_then(Value::as_array) {
            if tasks.is_empty() {
                return "No tasks found".to_string();
            }
            return tasks
                .iter()
                .filter_map(|task| {
                    let id = task.get("id").and_then(Value::as_str)?;
                    let status = task.get("status").and_then(Value::as_str)?;
                    let subject = task.get("subject").and_then(Value::as_str)?;
                    let owner = task
                        .get("owner")
                        .and_then(Value::as_str)
                        .map(|owner| format!(" ({owner})"))
                        .unwrap_or_default();
                    let blocked = task
                        .get("blockedBy")
                        .and_then(Value::as_array)
                        .filter(|items| !items.is_empty())
                        .map(|items| {
                            format!(
                                " [blocked by {}]",
                                items
                                    .iter()
                                    .filter_map(|value| {
                                        value.as_str().map(|id| format!("#{id}"))
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        })
                        .unwrap_or_default();
                    Some(format!("#{id} [{status}] {subject}{owner}{blocked}"))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    if tool_name == "WebFetch" {
        if let Some(result) = data.get("result").and_then(Value::as_str) {
            return result.to_string();
        }
    }
    if tool_name == "WebSearch" {
        if let Some(query) = data.get("query").and_then(Value::as_str) {
            let mut formatted = format!("Web search results for query: \"{query}\"\n\n");
            if let Some(results) = data.get("results").and_then(Value::as_array) {
                for result in results {
                    if result.is_null() {
                        continue;
                    }
                    if let Some(text) = result.as_str() {
                        formatted.push_str(text);
                        formatted.push_str("\n\n");
                    } else if result
                        .get("content")
                        .and_then(Value::as_array)
                        .map(|content| !content.is_empty())
                        .unwrap_or(false)
                    {
                        formatted.push_str("Links: ");
                        formatted.push_str(
                            &serde_json::to_string(result.get("content").unwrap())
                                .unwrap_or_else(|_| "[]".to_string()),
                        );
                        formatted.push_str("\n\n");
                    } else {
                        formatted.push_str("No links found.\n\n");
                    }
                }
            }
            formatted.push_str("\nREMINDER: You MUST include the sources above in your response to the user using markdown hyperlinks.");
            return formatted.trim().to_string();
        }
    }
    if tool_name == "Config" {
        if let Some(error) = data.get("error").and_then(Value::as_str) {
            return format!("Error: {error}");
        }
        let action = data
            .get("operation")
            .or_else(|| data.get("action"))
            .and_then(Value::as_str);
        let setting = data
            .get("setting")
            .or_else(|| data.get("key"))
            .and_then(Value::as_str);
        if let (Some(action), Some(setting)) = (action, setting) {
            if action == "get" {
                let value = data.get("value").unwrap_or(&Value::Null);
                return format!("{setting} = {}", json_stringify_for_ts(value));
            }
            if action == "set" {
                let value = data
                    .get("newValue")
                    .or_else(|| data.get("value"))
                    .unwrap_or(&Value::Null);
                return format!("Set {setting} to {}", json_stringify_for_ts(value));
            }
        }
    }
    if tool_name == "ListMcpResourcesTool" {
        if data
            .as_array()
            .is_some_and(|resources| resources.is_empty())
        {
            return "No resources found. MCP servers may still provide tools even if they have no resources.".to_string();
        }
        return json_stringify_for_ts(data);
    }
    if let Some(text) = data.as_str() {
        return text.to_string();
    }
    if data.get("type").and_then(Value::as_str) == Some("file_unchanged") {
        return "File unchanged since last read. The content from the earlier Read tool_result in this conversation is still current — refer to that instead of re-reading.".to_string();
    }
    if data.get("type").and_then(Value::as_str) == Some("text") {
        if let Some(file) = data.get("file") {
            if let Some(content) = file.get("content").and_then(Value::as_str) {
                let start_line =
                    file.get("startLine").and_then(Value::as_u64).unwrap_or(1) as usize;
                return add_line_numbers_ts(content, start_line);
            }
        }
        if let Some(content) = data.get("content").and_then(Value::as_str) {
            return content.to_string();
        }
    }
    if let Some(mode) = data.get("mode").and_then(Value::as_str) {
        let content = data.get("content").and_then(Value::as_str).unwrap_or("");
        let limit_info = format_search_limit_info(data);
        match mode {
            "content" => {
                let result = if content.is_empty() {
                    "No matches found".to_string()
                } else {
                    content.to_string()
                };
                return if let Some(limit_info) = limit_info {
                    format!("{result}\n\n[Showing results with pagination = {limit_info}]")
                } else {
                    result
                };
            }
            "count" => {
                let raw_content = if content.is_empty() {
                    "No matches found".to_string()
                } else {
                    content.to_string()
                };
                let matches = data.get("numMatches").and_then(Value::as_u64).unwrap_or(0);
                let files = data.get("numFiles").and_then(Value::as_u64).unwrap_or(0);
                let occurrence = if matches == 1 {
                    "occurrence"
                } else {
                    "occurrences"
                };
                let file = if files == 1 { "file" } else { "files" };
                let mut summary =
                    format!("\n\nFound {matches} total {occurrence} across {files} {file}.");
                if let Some(limit_info) = limit_info {
                    summary.push_str(&format!(" with pagination = {limit_info}"));
                }
                return raw_content + &summary;
            }
            _ => {
                let filenames = data
                    .get("filenames")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let files = data
                    .get("numFiles")
                    .and_then(Value::as_u64)
                    .unwrap_or(filenames.len() as u64);
                if files == 0 {
                    return "No files found".to_string();
                }
                let file = if files == 1 { "file" } else { "files" };
                let limit = limit_info
                    .map(|info| format!(" {info}"))
                    .unwrap_or_default();
                return format!("Found {files} {file}{limit}\n{}", filenames.join("\n"));
            }
        }
    }
    if let Some(filenames) = data.get("filenames").and_then(Value::as_array) {
        let mut lines = filenames
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        if data
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            lines.push(
                "(Results are truncated. Consider using a more specific path or pattern.)"
                    .to_string(),
            );
        }
        return if lines.is_empty() {
            "No files found".to_string()
        } else {
            lines.join("\n")
        };
    }
    if let Some(message) = data.get("message").and_then(Value::as_str) {
        return message.to_string();
    }
    if let Some(error) = data.get("error").and_then(Value::as_str) {
        return error.to_string();
    }
    data.to_string()
}

fn format_search_limit_info(data: &Value) -> Option<String> {
    let limit = data.get("appliedLimit").and_then(Value::as_u64);
    let offset = data
        .get("appliedOffset")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    limit.map(|limit| {
        if offset > 0 {
            format!("limit: {limit}, offset: {offset}")
        } else {
            format!("limit: {limit}")
        }
    })
}

fn json_stringify_for_ts(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tool_error_matches_ts_model_content() {
        assert_eq!(
            unknown_tool_error_text("MissingTool"),
            "Error: No such tool available: MissingTool"
        );
        assert_eq!(
            unknown_tool_error_content("MissingTool"),
            "<tool_use_error>Error: No such tool available: MissingTool</tool_use_error>"
        );
    }

    #[test]
    fn read_file_unchanged_maps_to_ts_stub() {
        let text = format_tool_result_for_model(
            "Read",
            &serde_json::json!({
                "type": "file_unchanged",
                "file": { "filePath": "/tmp/a.txt" }
            }),
        );
        assert_eq!(
            text,
            "File unchanged since last read. The content from the earlier Read tool_result in this conversation is still current — refer to that instead of re-reading."
        );
    }
}
