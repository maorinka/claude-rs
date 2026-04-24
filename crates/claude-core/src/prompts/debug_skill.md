# Debug Skill

Help the user debug an issue they're encountering in this current Claude Code session.
{{JUST_ENABLED_SECTION}}
## Session Debug Log

The debug log for the current session is at: `{{DEBUG_LOG_PATH}}`

{{LOG_INFO}}

For additional context, grep for [ERROR] and [WARN] lines across the full file.

## Issue Description

{{ISSUE_DESCRIPTION}}

## Settings

Remember that settings are in:
* user - {{USER_SETTINGS_PATH}}
* project - {{PROJECT_SETTINGS_PATH}}
* local - {{LOCAL_SETTINGS_PATH}}

## Instructions

1. Review the user's issue description
2. The last 20 lines show the debug file format. Look for [ERROR] and [WARN] entries, stack traces, and failure patterns across the file
3. Consider launching the claude-code-guide subagent to understand the relevant Claude Code features
4. Explain what you found in plain language
5. Suggest concrete fixes or next steps
