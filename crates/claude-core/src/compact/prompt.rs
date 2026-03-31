/// The prompt sent to Claude to summarize a conversation.
pub fn compact_prompt() -> String {
    r#"Your task is to create a detailed summary of the conversation so far, paying close attention to the user's explicit requests and your previous actions. This summary should be thorough in capturing technical details, code patterns, and architectural decisions that would be essential for continuing development work without losing context.

Please create a summary that includes:

1. **Primary Request and Intent**: What the user is trying to accomplish, including specific requirements and constraints they've mentioned.

2. **Key Technical Concepts**: List all important technical concepts, technologies, libraries, patterns, and architectural decisions discussed.

3. **Files and Code Sections**: Enumerate all files that have been created, modified, or discussed. For each file include relevant code snippets that were written or modified.

4. **Errors and Fixes**: Document any errors encountered and how they were resolved.

5. **Problem Solving**: Describe problems that were solved and any ongoing troubleshooting.

6. **All User Messages**: Reproduce ALL non-tool-result user messages to preserve the full conversation context.

7. **Pending Tasks**: List any explicitly requested tasks that haven't been completed yet.

8. **Current Work**: Describe what was being worked on immediately before this summary was requested.

9. **Optional Next Step**: If there's a clear next step, state it with direct quotes from the conversation.

CRITICAL: Respond with TEXT ONLY. Do NOT call any tools. Your entire response must be plain text summary."#.to_string()
}

/// Format the summary as a user message for the compacted conversation.
pub fn format_compact_user_message(summary: &str) -> String {
    format!(
        "This session is being continued from a previous conversation that ran out of context. \
         The summary below covers the earlier portion of the conversation.\n\n\
         {}\n\n\
         Continue the conversation from where we left off. Do not ask clarifying questions about \
         what was already discussed — the summary above contains all the context you need.",
        summary
    )
}
