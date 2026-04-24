// ── Compaction error messages ────────────────────────────────────────────────
// Mirrors the TS `compact.ts` error constants.

pub const ERROR_MESSAGE_NOT_ENOUGH_MESSAGES: &str = "Not enough messages to compact.";

pub const ERROR_MESSAGE_PROMPT_TOO_LONG: &str =
    "Conversation too long. Press esc twice to go up a few messages and try again.";

pub const ERROR_MESSAGE_USER_ABORT: &str = "API Error: Request was aborted.";

pub const ERROR_MESSAGE_INCOMPLETE_RESPONSE: &str =
    "Compaction interrupted \u{00b7} This may be due to network issues \u{2014} please try again.";

/// Synthetic marker injected when retrying compaction after a prompt-too-long error.
pub const PTL_RETRY_MARKER: &str = "[earlier conversation truncated for compaction retry]";

// ── Compaction prompt constants ─────────────────────────────────────────────

const NO_TOOLS_PREAMBLE: &str = "CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.\n\n\
- Do NOT use Read, Bash, Grep, Glob, Edit, Write, or ANY other tool.\n\
- You already have all the context you need in the conversation above.\n\
- Tool calls will be REJECTED and will waste your only turn \u{2014} you will fail the task.\n\
- Your entire response must be plain text: an <analysis> block followed by a <summary> block.\n\n";

const DETAILED_ANALYSIS_INSTRUCTION: &str = "Before providing your final summary, wrap your analysis in <analysis> tags to organize your thoughts and ensure you've covered all necessary points. In your analysis process:\n\n\
1. Chronologically analyze each message and section of the conversation. For each section thoroughly identify:\n\
   - The user's explicit requests and intents\n\
   - Your approach to addressing the user's requests\n\
   - Key decisions, technical concepts and code patterns\n\
   - Specific details like:\n\
     - file names\n\
     - full code snippets\n\
     - function signatures\n\
     - file edits\n\
   - Errors that you ran into and how you fixed them\n\
   - Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.\n\
2. Double-check for technical accuracy and completeness, addressing each required element thoroughly.";

const DETAILED_ANALYSIS_INSTRUCTION_PARTIAL: &str = "Before providing your final summary, wrap your analysis in <analysis> tags to organize your thoughts and ensure you've covered all necessary points. In your analysis process:\n\n\
1. Analyze the recent messages chronologically. For each section thoroughly identify:\n\
   - The user's explicit requests and intents\n\
   - Your approach to addressing the user's requests\n\
   - Key decisions, technical concepts and code patterns\n\
   - Specific details like:\n\
     - file names\n\
     - full code snippets\n\
     - function signatures\n\
     - file edits\n\
   - Errors that you ran into and how you fixed them\n\
   - Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.\n\
2. Double-check for technical accuracy and completeness, addressing each required element thoroughly.";

const NO_TOOLS_TRAILER: &str =
    "\n\nREMINDER: Do NOT call any tools. Respond with plain text only \u{2014} \
an <analysis> block followed by a <summary> block. \
Tool calls will be rejected and you will fail the task.";

/// The prompt sent to Claude to summarize a conversation.
/// Matches the TS prompt.ts structure: NO_TOOLS_PREAMBLE + body + NO_TOOLS_TRAILER,
/// with <analysis> scratchpad then <summary> output format.
pub fn compact_prompt() -> String {
    format!("{preamble}Your task is to create a detailed summary of the conversation so far, paying close attention to the user's explicit requests and your previous actions.
This summary should be thorough in capturing technical details, code patterns, and architectural decisions that would be essential for continuing development work without losing context.

{analysis}

Your summary should include the following sections:

1. Primary Request and Intent: Capture all of the user's explicit requests and intents in detail
2. Key Technical Concepts: List all important technical concepts, technologies, and frameworks discussed.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Pay special attention to the most recent messages and include full code snippets where applicable and include a summary of why this file read or edit is important.
4. Errors and fixes: List all errors that you ran into, and how you fixed them. Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All user messages: List ALL user messages that are not tool results. These are critical for understanding the users' feedback and changing intent.
7. Pending Tasks: Outline any pending tasks that you have explicitly been asked to work on.
8. Current Work: Describe in detail precisely what was being worked on immediately before this summary request, paying special attention to the most recent messages from both user and assistant. Include file names and code snippets where applicable.
9. Optional Next Step: List the next step that you will take that is related to the most recent work you were doing. IMPORTANT: ensure that this step is DIRECTLY in line with the user's most recent explicit requests, and the task you were working on immediately before this summary request. If your last task was concluded, then only list next steps if they are explicitly in line with the users request. Do not start on tangential requests or really old requests that were already completed without confirming with the user first.
                       If there is a next step, include direct quotes from the most recent conversation showing exactly what task you were working on and where you left off. This should be verbatim to ensure there's no drift in task interpretation.

Here's an example of how your output should be structured:

<example>
<analysis>
[Your thought process, ensuring all points are covered thoroughly and accurately]
</analysis>

<summary>
1. Primary Request and Intent:
   [Detailed description]

2. Key Technical Concepts:
   - [Concept 1]
   - [Concept 2]
   - [...]

3. Files and Code Sections:
   - [File Name 1]
      - [Summary of why this file is important]
      - [Summary of the changes made to this file, if any]
      - [Important Code Snippet]
   - [File Name 2]
      - [Important Code Snippet]
   - [...]

4. Errors and fixes:
    - [Detailed description of error 1]:
      - [How you fixed the error]
      - [User feedback on the error if any]
    - [...]

5. Problem Solving:
   [Description of solved problems and ongoing troubleshooting]

6. All user messages:
    - [Detailed non tool use user message]
    - [...]

7. Pending Tasks:
   - [Task 1]
   - [Task 2]
   - [...]

8. Current Work:
   [Precise description of current work]

9. Optional Next Step:
   [Optional Next step to take]

</summary>
</example>

Please provide your summary based on the conversation so far, following this structure and ensuring precision and thoroughness in your response.

There may be additional summarization instructions provided in the included context. If so, remember to follow these instructions when creating the above summary. Examples of instructions include:
<example>
## Compact Instructions
When summarizing the conversation focus on typescript code changes and also remember the mistakes you made and how you fixed them.
</example>

<example>
# Summary instructions
When you are using compact - please focus on test output and code changes. Include file reads verbatim.
</example>
{trailer}",
        preamble = NO_TOOLS_PREAMBLE,
        analysis = DETAILED_ANALYSIS_INSTRUCTION,
        trailer = NO_TOOLS_TRAILER,
    )
}

/// Partial compaction prompt — summarizes only the recent messages,
/// preserving earlier context intact.
/// Matches the TS `PARTIAL_COMPACT_PROMPT`.
pub fn partial_compact_prompt() -> String {
    format!("{preamble}Your task is to create a detailed summary of the RECENT portion of the conversation \u{2014} the messages that follow earlier retained context. The earlier messages are being kept intact and do NOT need to be summarized. Focus your summary on what was discussed, learned, and accomplished in the recent messages only.

{analysis}

Your summary should include the following sections:

1. Primary Request and Intent: Capture the user's explicit requests and intents from the recent messages
2. Key Technical Concepts: List important technical concepts, technologies, and frameworks discussed recently.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Include full code snippets where applicable and include a summary of why this file read or edit is important.
4. Errors and fixes: List errors encountered and how they were fixed.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All user messages: List ALL user messages from the recent portion that are not tool results.
7. Pending Tasks: Outline any pending tasks from the recent messages.
8. Current Work: Describe precisely what was being worked on immediately before this summary request.
9. Optional Next Step: List the next step related to the most recent work. Include direct quotes from the most recent conversation.

Here's an example of how your output should be structured:

<example>
<analysis>
[Your thought process, ensuring all points are covered thoroughly and accurately]
</analysis>

<summary>
1. Primary Request and Intent:
   [Detailed description]

2. Key Technical Concepts:
   - [Concept 1]
   - [Concept 2]
   - [...]

3. Files and Code Sections:
   - [File Name 1]
      - [Summary of why this file is important]
      - [Summary of the changes made to this file, if any]
      - [Important Code Snippet]
   - [File Name 2]
      - [Important Code Snippet]
   - [...]

4. Errors and fixes:
    - [Detailed description of error 1]:
      - [How you fixed the error]
      - [User feedback on the error if any]
    - [...]

5. Problem Solving:
   [Description of solved problems and ongoing troubleshooting]

6. All user messages:
    - [Detailed non tool use user message]
    - [...]

7. Pending Tasks:
   - [Task 1]
   - [Task 2]
   - [...]

8. Current Work:
   [Precise description of current work]

9. Optional Next Step:
   [Optional Next step to take]

</summary>
</example>

Please provide your summary based on the RECENT messages only (after the retained earlier context), following this structure and ensuring precision and thoroughness in your response.
{trailer}",
        preamble = NO_TOOLS_PREAMBLE,
        analysis = DETAILED_ANALYSIS_INSTRUCTION_PARTIAL,
        trailer = NO_TOOLS_TRAILER,
    )
}

/// Prefix-preserving partial compaction prompt — summarizes messages up to
/// a certain point, with the understanding that newer messages will follow.
/// Matches the TS `PARTIAL_COMPACT_UP_TO_PROMPT`.
pub fn partial_compact_up_to_prompt() -> String {
    format!("{preamble}Your task is to create a detailed summary of this conversation. This summary will be placed at the start of a continuing session; newer messages that build on this context will follow after your summary (you do not see them here). Summarize thoroughly so that someone reading only your summary and then the newer messages can fully understand what happened and continue the work.

{analysis}

Your summary should include the following sections:

1. Primary Request and Intent: Capture the user's explicit requests and intents in detail
2. Key Technical Concepts: List important technical concepts, technologies, and frameworks discussed.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Include full code snippets where applicable and include a summary of why this file read or edit is important.
4. Errors and fixes: List errors encountered and how they were fixed.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All user messages: List ALL user messages that are not tool results.
7. Pending Tasks: Outline any pending tasks.
8. Work Completed: Describe what was accomplished by the end of this portion.
9. Context for Continuing Work: Summarize any context, decisions, or state that would be needed to understand and continue the work in subsequent messages.

Here's an example of how your output should be structured:

<example>
<analysis>
[Your thought process, ensuring all points are covered thoroughly and accurately]
</analysis>

<summary>
1. Primary Request and Intent:
   [Detailed description]

2. Key Technical Concepts:
   - [Concept 1]
   - [Concept 2]
   - [...]

3. Files and Code Sections:
   - [File Name 1]
      - [Summary of why this file is important]
      - [Important Code Snippet]
   - [...]

4. Errors and fixes:
    - [Detailed description of error]:
      - [How you fixed the error]
    - [...]

5. Problem Solving:
   [Description of solved problems and ongoing troubleshooting]

6. All user messages:
    - [Detailed non tool use user message]
    - [...]

7. Pending Tasks:
   - [Task 1]
   - [...]

8. Work Completed:
   [Description of what was accomplished]

9. Context for Continuing Work:
   [Context needed to continue]

</summary>
</example>

Please provide your summary following this structure, ensuring precision and thoroughness in your response.
{trailer}",
        preamble = NO_TOOLS_PREAMBLE,
        analysis = DETAILED_ANALYSIS_INSTRUCTION,
        trailer = NO_TOOLS_TRAILER,
    )
}

/// Options for building the post-compaction user message.
/// Matches the TS `getCompactUserSummaryMessage` parameters.
pub struct CompactUserMessageOptions<'a> {
    /// The formatted compaction summary.
    pub summary: &'a str,
    /// Optional path to the full transcript file.
    pub transcript_path: Option<&'a str>,
    /// Whether recent messages are preserved verbatim after the summary.
    pub recent_messages_preserved: bool,
    /// Whether to suppress follow-up questions and instruct Claude to continue directly.
    pub suppress_follow_up_questions: bool,
    /// Whether running in autonomous/proactive mode.
    pub proactive_mode: bool,
}

/// Format the summary as a user message for the compacted conversation.
/// Matches the TS `getCompactUserSummaryMessage` from `prompt.ts`.
pub fn format_compact_user_message(opts: &CompactUserMessageOptions) -> String {
    let mut msg = format!(
        "This session is being continued from a previous conversation that ran out of context. \
         The summary below covers the earlier portion of the conversation.\n\n{}",
        opts.summary
    );

    if let Some(path) = opts.transcript_path {
        msg.push_str(&format!(
            "\n\nIf you need specific details from before compaction \
             (like exact code snippets, error messages, or content you generated), \
             read the full transcript at: {}",
            path
        ));
    }

    if opts.recent_messages_preserved {
        msg.push_str("\n\nRecent messages are preserved verbatim.");
    }

    if opts.suppress_follow_up_questions {
        msg.push_str(
            "\n\nContinue the conversation from where it left off without asking the user \
             any further questions. Resume directly \u{2014} do not acknowledge the summary, \
             do not recap what was happening, do not preface with \"I'll continue\" or similar. \
             Pick up the last task as if the break never happened.",
        );
    } else {
        msg.push_str(
            "\n\nContinue the conversation from where we left off. \
             Do not ask clarifying questions about what was already discussed \u{2014} \
             the summary above contains all the context you need.",
        );
    }

    if opts.proactive_mode {
        msg.push_str(
            "\n\nYou are running in autonomous/proactive mode. \
             This is NOT a first wake-up \u{2014} you were already working autonomously \
             before compaction. Continue your work loop: pick up where you left off \
             based on the summary above. Do not greet the user or ask what to work on.",
        );
    }

    msg
}

/// Simple format for the common case (backward compat).
pub fn format_compact_user_message_simple(summary: &str) -> String {
    format_compact_user_message(&CompactUserMessageOptions {
        summary,
        transcript_path: None,
        recent_messages_preserved: false,
        suppress_follow_up_questions: false,
        proactive_mode: false,
    })
}
