// src/main.rs
mod workarea;

use ai_agent::session::SessionMetadata;
use ai_agent::tools::get_all_base_tools;
use ai_agent::{Agent, AgentEvent, ContentDelta, EnvConfig, ExitReason, Message, MessageRole};
use colored::Colorize;
use futures::StreamExt;
use std::io;
use workarea::{WorkArea, WorkAreaEvent};

const AI_ICON: &str = "●";
const AI_ICON_PADDING: &str = " ";

/// Accumulates streaming text deltas and returns complete lines as they arrive.
struct LineBuffer {
    partial: String,
}

impl LineBuffer {
    fn new() -> Self {
        Self {
            partial: String::new(),
        }
    }

    /// Append text and return any complete lines.
    fn append(&mut self, text: &str) -> Vec<String> {
        self.partial.push_str(text);
        if !self.partial.contains('\n') {
            return Vec::new();
        }
        let mut lines = Vec::new();
        let mut current = String::new();
        for ch in self.partial.chars() {
            if ch == '\n' {
                lines.push(std::mem::take(&mut current));
            } else if ch != '\r' {
                current.push(ch);
            }
        }
        self.partial = current;
        lines
    }

    /// Return any remaining partial text (consumes the buffer).
    fn drain(&mut self) -> Option<String> {
        if self.partial.is_empty() {
            None
        } else {
            Some(self.partial.drain(..).collect())
        }
    }
}

fn exit_reason_label(reason: &ExitReason) -> String {
    match reason {
        ExitReason::Completed => String::from("done"),
        ExitReason::MaxTurns { .. } => String::from("max turns"),
        ExitReason::AbortedStreaming { .. } => String::from("aborted"),
        ExitReason::AbortedTools { .. } => String::from("aborted"),
        ExitReason::HookStopped => String::from("hook stopped"),
        ExitReason::StopHookPrevented => String::from("hook blocked"),
        ExitReason::PromptTooLong { .. } => String::from("prompt too long"),
        ExitReason::ImageError { error: _ } => String::from("image error"),
        ExitReason::ModelError { error: _ } => String::from("model error"),
        ExitReason::BlockingLimit => String::from("blocking limit"),
        ExitReason::TokenBudgetExhausted { .. } => String::from("budget exhausted"),
        ExitReason::MaxTokens => String::from("max tokens"),
    }
}
/// Extract a short display argument from a tool's JSON input.
/// Mirrors TS `renderToolUseMessage` for common base tools when render
/// metadata is not available (most tools register without it).
fn tool_input_summary(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    let obj = input.as_object()?;
    match tool_name {
        "Bash" => obj.get("command").and_then(|v| v.as_str()).map(String::from),
        "FileRead" | "FileWrite" => obj.get("path").and_then(|v| v.as_str()).map(String::from),
        "FileEdit" => obj.get("file_path").and_then(|v| v.as_str()).map(String::from),
        "Glob" => obj.get("path").and_then(|v| v.as_str()).map(String::from),
        "Grep" => obj.get("pattern").and_then(|v| v.as_str()).map(String::from),
        "WebFetch" | "WebBrowser" => obj.get("url").and_then(|v| v.as_str()).map(String::from),
        "WebSearch" => obj.get("query").and_then(|v| v.as_str()).map(String::from),
        "NotebookEdit" => obj.get("notebook_path").and_then(|v| v.as_str()).map(String::from),
        "TaskCreate" => obj.get("subject").and_then(|v| v.as_str()).map(String::from),
        "TaskUpdate" => obj.get("taskId").and_then(|v| v.as_str()).map(String::from),
        "TaskGet" => obj.get("taskId").and_then(|v| v.as_str()).map(String::from),
        "Skill" => obj.get("skill").and_then(|v| v.as_str()).map(String::from),
        _ => None,
    }
}

/// Map internal tool names to TS userFacingName equivalents.
fn tool_display_name(tool_name: &str) -> String {
    match tool_name {
        "Bash" => "Bash".to_string(),
        "FileRead" => "Read".to_string(),
        "FileWrite" => "Write".to_string(),
        "FileEdit" => "Edit".to_string(),
        "Glob" => "Glob".to_string(),
        "Grep" => "Grep".to_string(),
        "Skill" => "Skill".to_string(),
        "Monitor" => "Monitor".to_string(),
        "send_user_file" => "Send User File".to_string(),
        "WebBrowser" => "Web Browser".to_string(),
        "WebFetch" => "Web Fetch".to_string(),
        "WebSearch" => "Web Search".to_string(),
        "NotebookEdit" => "Notebook Edit".to_string(),
        "TaskCreate" => "Create Task".to_string(),
        "TaskList" => "Task List".to_string(),
        "TaskUpdate" => "Update Task".to_string(),
        "TaskGet" => "Get Task".to_string(),
        _ => tool_name.to_string(),
    }
}

/// Format a `ToolStart` event.
/// TS format: `TOOL_NAME (arguments)`
///
/// Most tools in ai-agent register without render metadata, so
/// `display_name`/`summary`/`activity_description` are `None`. We fall back to
/// deriving the display from `tool_name` and the `input` JSON.
///
/// Returns just the text (no icon) — the TUI adds its own decoration.
fn format_tool_start(
    tool_name: &str,
    input: &serde_json::Value,
    display_name: &Option<String>,
    summary: &Option<String>,
    activity_description: &Option<String>,
) -> String {
    // First try: explicit render metadata from ToolRenderFns
    let dn = display_name.as_deref().unwrap_or_default();
    let sm = summary.as_deref().unwrap_or_default();
    let ad = activity_description.as_deref().unwrap_or_default();

    if !dn.is_empty() && !sm.is_empty() {
        return format!("{dn} ({sm})");
    }
    if !dn.is_empty() {
        return if !ad.is_empty() && ad != dn {
            format!("{dn} ({ad})")
        } else {
            format!("{dn}")
        };
    }

    // Second: derive from tool_name + input
    let name = tool_display_name(tool_name);
    if let Some(args) = tool_input_summary(tool_name, input) {
        return format!("{name} ({args})");
    }

    name
}

/// Format a `ToolComplete` event.
/// TS: tool.renderToolResultMessage() -> "Read 42 lines", etc.
/// Returns `None` when there is no rendered result to display.
fn format_tool_complete(rendered_result: &Option<String>) -> Option<String> {
    rendered_result.as_ref().map(|r| format!("  {}", r.trim()))
}

/// Format a `ToolError` event.
/// TS: red "Error: ..." message
fn format_tool_error(error: &str) -> String {
    let msg = error.strip_prefix("Error:").unwrap_or(error).trim();
    format!("  Error: {}", msg)
}

#[allow(unused_assignments)]
fn main() -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local_set = tokio::task::LocalSet::new();
    local_set.block_on(&runtime, async {
        let workarea = WorkArea::new()?;

        // Load config and create agent
        let config = EnvConfig::load();
        let model = config.model.as_deref().unwrap_or("sonnet");
        let agent = Agent::new(model)
            .max_turns(u32::MAX)
            .tools(get_all_base_tools())
            .system_prompt(
                "You are an AI assistant. You have access to tools for executing commands, \
                 reading and editing files, and more. Be helpful and concise.",
            );

        let session_agent = agent.clone();
        let interrupt_agent = agent.clone();
        let model_agent = agent.clone();

        let _ = workarea.print("[main] starting event loop");

        let mut line_buffer = LineBuffer::new();
        let mut line_number: usize = 0;
        let mut messages: Vec<Message> = Vec::new();

        // Create a single event subscriber for the session lifetime.
        // Events flow while the guard is kept alive.
        let (sub, _guard) = agent.subscribe();
        tokio::pin!(sub);

        // Track active query handle; None means idle
        let mut query_active = false;

        loop {
            if query_active {
                // Agent is running — accept keyboard and agent events
                tokio::select! {
                    // Keyboard input (prioritized for responsive TUI)
                    tick_result = workarea.tick() => {
                        match tick_result {
                            Ok(WorkAreaEvent::Interrupt) => {
                                interrupt_agent.interrupt();
                                workarea.set_status(String::from("interrupted"));
                            }
                            Ok(WorkAreaEvent::Exit) => {
                                query_active = false;
                                workarea.set_status(String::from("exiting..."));
                                break;
                            }
                            Ok(WorkAreaEvent::Submit(line)) => {
                                // New prompt during active query — cancel current query
                                query_active = false;

                                workarea.set_phase(workarea::Phase::Processing);
                                workarea.set_status(String::from("thinking..."));
                                messages.push(Message {
                                    role: MessageRole::User,
                                    content: line.clone(),
                                    ..Message::default()
                                });

                                let ag = session_agent.clone();
                                let handle = tokio::task::spawn_local(async move {
                                    ag.query(&line).await.unwrap_or_else(|e| {
                                        eprintln!("query error: {e}");
                                        ai_agent::QueryResult {
                                            text: String::new(),
                                            exit_reason: ai_agent::ExitReason::ModelError {
                                                error: e.to_string(),
                                            },
                                            usage: Default::default(),
                                            num_turns: 0,
                                            duration_ms: 0,
                                        }
                                    })
                                });
                                query_active = true;
                                // Store the handle in a task-local for cleanup;
                                // dropping the handle on cancel just cancels the future,
                                // the agent handles abort via interrupt()
                                let _ = handle;
                            }
                            Err(e) => {
                                eprintln!("workarea error: {e}");
                                query_active = false;
                                break;
                            }
                        }
                    }

                    // Agent events (from event subscriber stream)
                    agent_event = sub.next() => {
                        if let Some(event) = agent_event {
                            match event {
                                AgentEvent::MessageStart { message_id } => {
                                    let _ = workarea.print(format!("[message start: {message_id}]"));
                                }
                                AgentEvent::ContentBlockDelta { delta, .. } => {
                                    match delta {
                                        ContentDelta::Text { text } => {
                                            let lines = line_buffer.append(&text);
                                            for line in lines {
                                                if line.trim().is_empty() && line_number == 0 {
                                                    continue;
                                                }
                                                line_number += 1;
                                                let prefix = if line_number == 1 {
                                                    AI_ICON.bright_green().to_string()
                                                } else {
                                                    AI_ICON_PADDING.to_string()
                                                };
                                                let line = format!("{} {}", prefix, line);
                                                let _ = workarea.print(line);
                                            }
                                        }
                                        ContentDelta::Thinking { .. } => {}
                                        ContentDelta::ToolUse { .. } => {}
                                    }
                                }
                                AgentEvent::MessageStop {} => {
                                    let _ = workarea.print(format!("[message stop]"));
                                    if let Some(text) = line_buffer.drain() {
                                        if !text.trim().is_empty() {
                                            line_number += 1;
                                            let prefix = if line_number == 1 {
                                                AI_ICON.bright_green().to_string()
                                            } else {
                                                AI_ICON_PADDING.to_string()
                                            };
                                            let line = format!("{} {}", prefix, text);
                                            let _ = workarea.print(line);
                                        }
                                    }
                                    line_number = 0;
                                    let _ = workarea.print("\n");
                                }
                                AgentEvent::ToolStart {
                                    tool_name,
                                    input,
                                    display_name,
                                    summary,
                                    activity_description,
                                    ..
                                } => {
                                    let _ = workarea.print(format!("{} {}", AI_ICON, format_tool_start(&tool_name, &input, &display_name, &summary, &activity_description)));
                                }
                                AgentEvent::ToolComplete { rendered_result, .. } => {
                                    if let Some(line) = format_tool_complete(&rendered_result) {
                                        let _ = workarea.print(line);
                                    }
                                }
                                AgentEvent::ToolError { error, .. } => {
                                    let _ = workarea.print(format_tool_error(&error));
                                }
                                AgentEvent::Done { result } => {
                                    let _ = workarea.print(format!("[agent done: {}ms]", result.duration_ms));
                                    let reason = exit_reason_label(&result.exit_reason);
                                    workarea.set_status(format!(
                                        "{}  {}t in / {}t out  {} turns",
                                        reason,
                                        result.usage.input_tokens,
                                        result.usage.output_tokens,
                                        result.num_turns,
                                    ));
                                    let asm = session_agent
                                        .get_messages()
                                        .last()
                                        .filter(|m| m.role == MessageRole::Assistant)
                                        .cloned()
                                        .unwrap_or_default();
                                    messages.push(asm);
                                    query_active = false;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            } else {
                // Idle — only keyboard input
                let tick_result = workarea.tick().await;
                match tick_result {
                    Ok(WorkAreaEvent::Submit(line)) => {
                        workarea.set_phase(workarea::Phase::Processing);
                        workarea.set_status(String::from("thinking..."));
                        messages.push(Message {
                            role: MessageRole::User,
                            content: line.clone(),
                            ..Message::default()
                        });

                        let ag = session_agent.clone();
                        let handle = tokio::task::spawn_local(async move {
                            ag.query(&line).await.unwrap_or_else(|e| {
                                eprintln!("query error: {e}");
                                ai_agent::QueryResult {
                                    text: String::new(),
                                    exit_reason: ai_agent::ExitReason::ModelError {
                                        error: e.to_string(),
                                    },
                                    usage: Default::default(),
                                    num_turns: 0,
                                    duration_ms: 0,
                                }
                            })
                        });
                        query_active = true;
                        let _ = handle;
                    }
                    Ok(WorkAreaEvent::Exit) => {
                        workarea.set_status(String::from("exiting..."));
                        break;
                    }
                    Ok(WorkAreaEvent::Interrupt) => {
                        // No running agent, nothing to interrupt
                    }
                    Err(e) => {
                        eprintln!("workarea error: {e}");
                        break;
                    }
                }
            }

            // Return to input phase when no agent is running
            if !query_active {
                workarea.set_phase(workarea::Phase::Input);
                workarea.redraw();
            }
        }

        // Save session
        let meta = SessionMetadata {
            id: session_agent.get_session_id(),
            cwd: std::env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(String::from))
                .unwrap_or_default(),
            model: model_agent.get_model(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            message_count: messages.len() as u32,
            summary: None,
            tag: None,
        };
        if let Err(e) = ai_agent::session::save_session(
            &session_agent.get_session_id(),
            messages,
            Some(meta),
        )
        .await
        {
            eprintln!("warning: failed to save session: {e}");
        }

        Ok(())
    })
}
