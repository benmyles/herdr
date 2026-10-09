//! `agent-title prompt-submit`: the prompt-submit hook that keeps a Claude
//! Code agent's title in step with its work. Claude Code titles a session once,
//! from its first prompt, so a long conversation keeps that title through every
//! later task. After each substantial prompt this asks Claude Haiku whether the
//! work moved on and, when it did, reports a new title for the pane.
//!
//! The hook returns at once and prints nothing, since Claude Code adds a
//! prompt hook's output to the prompt: a detached `agent-title generate`
//! worker asks the model, so no prompt waits on it. Both stay silent and
//! succeed outside Herdr panes, without the `claude` command, or with
//! `ui.agents.live_titles` off.

use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::api::schema::{Method, PaneReportMetadataParams, PaneTarget, Request};

/// The metadata source the titles report as.
const TITLE_SOURCE: &str = "herdr:claude-title";
const PROMPT_ENV: &str = "HERDR_AGENT_TITLE_PROMPT";
const SEQ_ENV: &str = "HERDR_AGENT_TITLE_SEQ";
/// The prompt text the model sees, enough to name a task.
const MAX_PROMPT_CHARS: usize = 2000;
const MODEL_TIMEOUT: Duration = Duration::from_secs(60);
const SYSTEM_PROMPT: &str = "You name a coding agent's session after the work it is doing. \
Reply with the title only: 2 to 6 words, sentence case, no quotes, no trailing punctuation. \
When the user's new message continues the work the current title names, reply with the \
current title unchanged.";

pub(super) fn run_agent_title_command(args: &[String]) -> std::io::Result<i32> {
    match args {
        [command] if command == "prompt-submit" => {
            let mut input = String::new();
            // Drain the hook input so the agent never blocks on a full pipe.
            let _ = std::io::stdin().read_to_string(&mut input);
            prompt_submit(&input);
            Ok(0)
        }
        [command] if command == "generate" => {
            generate();
            Ok(0)
        }
        _ => {
            eprintln!("usage: herdr-benmyles agent-title prompt-submit");
            Ok(2)
        }
    }
}

fn pane_id() -> Option<String> {
    std::env::var(crate::integration::HERDR_PANE_ID_ENV_VAR)
        .ok()
        .filter(|pane_id| !pane_id.is_empty())
}

fn prompt_submit(input: &str) {
    if pane_id().is_none() {
        return;
    }
    let Some(prompt) = titled_prompt(input) else {
        return;
    };
    let enabled = crate::config::load_live_config()
        .map(|loaded| loaded.config.ui.agents.live_titles)
        .unwrap_or(true);
    if !enabled || !crate::integration::command_available("claude") {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // Prompts reach the worker in order of submission, so a slow answer about
    // an earlier prompt never replaces a newer title.
    let seq = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos().min(u128::from(u64::MAX)) as u64);
    let mut worker = crate::noninteractive_process::command(exe);
    worker
        .args(["agent-title", "generate"])
        .env(PROMPT_ENV, prompt)
        .env(SEQ_ENV, seq.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::platform::detach_from_process_group(&mut worker);
    let _ = worker.spawn();
}

/// The prompt worth titling from prompt-submit hook `input`: something the
/// user typed with a few words in it. Slash commands and messages Claude Code
/// submits itself, such as task notifications, keep the title.
fn titled_prompt(input: &str) -> Option<String> {
    let input = serde_json::from_str::<Value>(input).ok()?;
    if input.get("hook_event_name").and_then(Value::as_str) != Some("UserPromptSubmit") {
        return None;
    }
    let prompt = input.get("prompt").and_then(Value::as_str)?.trim();
    if prompt.starts_with(['/', '<']) || prompt.split_whitespace().count() < 3 {
        return None;
    }
    Some(prompt.chars().take(MAX_PROMPT_CHARS).collect())
}

fn generate() {
    let (Some(pane_id), Ok(prompt), Some(seq)) = (
        pane_id(),
        std::env::var(PROMPT_ENV),
        std::env::var(SEQ_ENV)
            .ok()
            .and_then(|seq| seq.parse::<u64>().ok()),
    ) else {
        return;
    };
    let current = current_title(&pane_id);
    let Some(answer) = ask_model(&title_request(current.as_deref(), &prompt)) else {
        return;
    };
    let Some(title) = new_title(&answer, current.as_deref()) else {
        return;
    };
    let _ = super::send_request_unchecked(&Request {
        id: "agent-title".into(),
        method: Method::PaneReportMetadata(PaneReportMetadataParams {
            pane_id,
            source: TITLE_SOURCE.into(),
            agent: Some("claude".into()),
            applies_to_source: None,
            title: Some(title),
            display_agent: None,
            state_labels: Default::default(),
            tokens: Default::default(),
            clear_title: false,
            clear_display_agent: false,
            clear_state_labels: false,
            seq: Some(seq),
            ttl_ms: None,
        }),
    });
}

/// The title the pane shows now: one reported for it, else the agent's own.
fn current_title(pane_id: &str) -> Option<String> {
    let response = super::send_request_unchecked(&Request {
        id: "agent-title:pane".into(),
        method: Method::PaneGet(PaneTarget {
            pane_id: pane_id.to_owned(),
        }),
    })
    .ok()?;
    let pane = response.pointer("/result/pane")?;
    ["title", "terminal_title_stripped"]
        .into_iter()
        .filter_map(|field| pane.get(field).and_then(Value::as_str))
        .map(str::trim)
        .find(|title| !title.is_empty())
        .map(str::to_owned)
}

fn title_request(current: Option<&str>, prompt: &str) -> String {
    format!(
        "Current title: {}\n\nThe user's new message:\n{prompt}",
        current.unwrap_or("none")
    )
}

/// Runs Claude Haiku once on `request` with Claude Code's own login. It runs
/// outside the project, without tools, MCP servers, hooks, or a saved
/// session, and without the Herdr pane environment, so it never reports to
/// the pane it titles.
fn ask_model(request: &str) -> Option<String> {
    let args = [
        "-p",
        "--model",
        "haiku",
        "--no-session-persistence",
        "--tools",
        "",
        "--strict-mcp-config",
        "--settings",
        r#"{"disableAllHooks":true}"#,
        "--system-prompt",
        SYSTEM_PROMPT,
        request,
    ]
    .map(str::to_owned);
    let mut command =
        crate::plugin_command::command_for_argv_in_dir("claude", &args, &std::env::temp_dir());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("HERDR") || key == "CLAUDECODE" {
            command.env_remove(key);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + MODEL_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(Some(_)) => return None,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut answer = String::new();
    child.stdout.take()?.read_to_string(&mut answer).ok()?;
    Some(answer)
}

/// The title in the model's `answer`, unless it is unusable or `current`.
fn new_title(answer: &str, current: Option<&str>) -> Option<String> {
    let line = answer
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let title = line
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '*'))
        .trim()
        .trim_end_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let words = title.split_whitespace().count();
    if title.is_empty() || words > 10 || title.chars().count() > 80 {
        return None;
    }
    (!current.is_some_and(|current| current.trim().eq_ignore_ascii_case(&title))).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(prompt: &str) -> String {
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "s1",
            "prompt": prompt,
        })
        .to_string()
    }

    #[test]
    fn typed_prompts_with_a_few_words_are_titled() {
        assert_eq!(
            titled_prompt(&input("  add Open in Editor to the menus  ")).as_deref(),
            Some("add Open in Editor to the menus")
        );
        let long = "word ".repeat(1000);
        assert_eq!(
            titled_prompt(&input(&long)).map(|prompt| prompt.chars().count()),
            Some(MAX_PROMPT_CHARS)
        );
    }

    #[test]
    fn short_replies_commands_and_injected_messages_keep_the_title() {
        for prompt in [
            "yes please",
            "commit",
            "/review the auth flow",
            "<task-notification>done</task-notification>",
        ] {
            assert_eq!(titled_prompt(&input(prompt)), None, "{prompt}");
        }
        let other_event = r#"{"hook_event_name":"Stop","prompt":"add Open in Editor now"}"#;
        assert_eq!(titled_prompt(other_event), None);
        assert_eq!(titled_prompt("not json"), None);
    }

    #[test]
    fn the_request_names_the_current_title() {
        assert_eq!(
            title_request(Some("Grid zoom fix"), "now add an editor setting"),
            "Current title: Grid zoom fix\n\nThe user's new message:\nnow add an editor setting"
        );
        assert!(title_request(None, "x").starts_with("Current title: none\n"));
    }

    #[test]
    fn model_answers_become_clean_titles_unless_unchanged_or_unusable() {
        assert_eq!(
            new_title("\n\"Open in Editor menu option.\"\n", Some("Grid zoom fix")).as_deref(),
            Some("Open in Editor menu option")
        );
        assert_eq!(
            new_title("**Agent  title  refresh**", None).as_deref(),
            Some("Agent title refresh")
        );
        assert_eq!(new_title("grid zoom fix", Some("Grid zoom fix")), None);
        assert_eq!(new_title("   \n", None), None);
        assert_eq!(new_title(&"word ".repeat(20), None), None, "not a title");
    }
}
