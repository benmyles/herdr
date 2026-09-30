//! `agent-context session-start`: the session-start hook that gives an agent
//! the context of the space it starts in. Claude Code and Codex run it from
//! their hook settings. It prints nothing, and succeeds, outside Herdr panes or
//! when the space has no context to give, so a hook can never block an agent.

use std::io::Read;

use serde_json::Value;

use crate::api::schema::{Method, PaneTarget, Request};

pub(super) fn run_agent_context_command(args: &[String]) -> std::io::Result<i32> {
    match args {
        [command] if command == "session-start" => {
            let mut input = String::new();
            // Drain the hook input so the agent never blocks on a full pipe.
            let _ = std::io::stdin().read_to_string(&mut input);
            if let Some(output) = session_start_output(&input, pane_id().as_deref(), fetch_context)
            {
                println!("{output}");
            }
            Ok(0)
        }
        _ => {
            eprintln!("usage: herdr-benmyles agent-context session-start");
            Ok(2)
        }
    }
}

fn pane_id() -> Option<String> {
    std::env::var(crate::integration::HERDR_PANE_ID_ENV_VAR)
        .ok()
        .filter(|pane_id| !pane_id.is_empty())
}

fn fetch_context(pane_id: &str) -> Option<String> {
    let request = Request {
        id: "agent-context".into(),
        method: Method::PaneAgentContextGet(PaneTarget {
            pane_id: pane_id.to_owned(),
        }),
    };
    let response = super::send_request_unchecked(&request).ok()?;
    response
        .pointer("/result/context")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The hook output for `input`: the space context as the additional context
/// Claude Code and Codex read from a session-start hook, or `None`.
fn session_start_output(
    input: &str,
    pane_id: Option<&str>,
    fetch: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    let pane_id = pane_id?;
    let event = serde_json::from_str::<Value>(input).ok().and_then(|input| {
        input
            .get("hook_event_name")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    if event
        .as_deref()
        .is_some_and(|event| event != "SessionStart")
    {
        return None;
    }
    let context = fetch(pane_id)?;
    Some(
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            }
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_the_context_as_session_start_hook_output() {
        let output = session_start_output(
            r#"{"hook_event_name":"SessionStart","source":"startup"}"#,
            Some("w1:p1"),
            |pane_id| {
                assert_eq!(pane_id, "w1:p1");
                Some("space context".into())
            },
        )
        .expect("output");
        let output: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            output,
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": "space context",
                }
            })
        );
    }

    #[test]
    fn stays_silent_outside_panes_other_events_and_spaces_without_context() {
        let never = |_: &str| -> Option<String> { panic!("no request expected") };
        assert_eq!(session_start_output("{}", None, never), None);
        assert_eq!(
            session_start_output(r#"{"hook_event_name":"Stop"}"#, Some("p"), never),
            None
        );
        assert_eq!(session_start_output("", Some("p"), |_| None), None);
        assert!(session_start_output("not json", Some("p"), |_| Some("x".into())).is_some());
    }
}
