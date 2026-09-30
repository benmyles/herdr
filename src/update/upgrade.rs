//! In-place upgrade: running servers hand their live panes to this binary, so
//! a freshly installed build takes over without stopping pane processes.

use super::*;

/// What happened to one running server.
enum UpgradeOutcome {
    Upgraded,
    TooOldForHandoff,
    Failed { error: String, still_running: bool },
}

/// Hands the live panes of every running server this command targets to the
/// executing binary: only the selected server with `--session` or a socket
/// override, else every running session. Returns whether all of them now run
/// this binary.
pub(crate) fn upgrade_running_servers() -> Result<bool, String> {
    let import_exe = env::current_exe()
        .map_err(|err| format!("failed to determine this binary's path: {err}"))?;
    let version = crate::build_info::version();
    let mut all_upgraded = true;
    let mut any_running = false;
    for target in running_update_targets()? {
        let status =
            crate::api::read_runtime_status_at(&target.socket_path, SERVER_STOP_RESPONSE_TIMEOUT)
                .map_err(|err| {
                format!(
                    "failed to read status for {} at {}: {err}",
                    target.label,
                    target.socket_path.display()
                )
            })?;
        let Some(status) = status else {
            if target.must_be_running {
                all_upgraded = false;
                eprintln!(
                    "{} looked running, but its status API did not respond at {}; stop it with `{}`.",
                    target.label,
                    target.socket_path.display(),
                    target.stop_command
                );
            }
            continue;
        };
        any_running = true;
        eprintln!(
            "upgrading {} (server v{}) to {}...",
            target.label,
            version_label(status.version.as_deref()),
            import_exe.display()
        );
        match upgrade_server_at(
            &target.socket_path,
            &status,
            &import_exe,
            &version,
            SERVER_HANDOFF_REQUEST_TIMEOUT,
            SERVER_HANDOFF_CONFIRM_TIMEOUT,
        ) {
            UpgradeOutcome::Upgraded => {
                let attach = target
                    .attach_command
                    .as_deref()
                    .unwrap_or(crate::build_info::BIN_NAME);
                eprintln!(
                    "{} now runs this build; pane processes kept running. attached clients exited: run `{attach}` to reattach with the new client.",
                    target.label
                );
            }
            UpgradeOutcome::TooOldForHandoff => {
                all_upgraded = false;
                eprintln!(
                    "{} runs a server too old for live handoff; stop it with `{}` (this exits its pane processes), then start it again.",
                    target.label, target.stop_command
                );
            }
            UpgradeOutcome::Failed {
                error,
                still_running,
            } => {
                all_upgraded = false;
                eprintln!("live handoff failed for {}: {error}", target.label);
                if still_running {
                    eprintln!(
                        "{} is still running; its panes were not touched.",
                        target.label
                    );
                } else {
                    eprintln!(
                        "no server is responding for {}; server log: {}",
                        target.label,
                        crate::session::data_dir()
                            .join("herdr-server.log")
                            .display()
                    );
                }
            }
        }
    }
    if !any_running && all_upgraded {
        eprintln!(
            "no running {} server to upgrade.",
            crate::build_info::BIN_NAME
        );
    }
    Ok(all_upgraded)
}

/// Hands one server's live panes to `import_exe` and waits until the server
/// answering on its socket is that build.
fn upgrade_server_at(
    socket_path: &Path,
    status: &crate::api::RuntimeStatus,
    import_exe: &Path,
    version: &str,
    request_timeout: Duration,
    confirm_timeout: Duration,
) -> UpgradeOutcome {
    use crate::api::schema::{Method, ServerLiveHandoffParams};

    if !server_supports_live_handoff(status) {
        return UpgradeOutcome::TooOldForHandoff;
    }
    let params = ServerLiveHandoffParams {
        import_exe: Some(import_exe.display().to_string()),
        expected_protocol: Some(crate::protocol::PROTOCOL_VERSION),
        expected_version: Some(version.to_string()),
    };
    let result = send_server_update_method_at(
        socket_path,
        request_timeout,
        "upgrade:server:live-handoff",
        Method::ServerLiveHandoff(params),
        "server live handoff",
    )
    .and_then(|()| {
        wait_for_running_server_protocol_at(
            socket_path,
            confirm_timeout,
            Some(crate::protocol::PROTOCOL_VERSION),
            Some(version),
        )
    });
    match result {
        Ok(()) => UpgradeOutcome::Upgraded,
        Err(error) => UpgradeOutcome::Failed {
            error,
            still_running: matches!(
                crate::api::read_runtime_status_at(socket_path, SERVER_STOP_RESPONSE_TIMEOUT),
                Ok(Some(_))
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn socket_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from(format!(
            "/tmp/hup-{name}-{}-{nanos}.sock",
            std::process::id()
        ))
    }

    fn status(live_handoff: bool) -> crate::api::RuntimeStatus {
        crate::api::RuntimeStatus {
            version: Some("0.9.1".into()),
            protocol: Some(crate::protocol::PROTOCOL_VERSION),
            capabilities: Some(
                serde_json::from_value(serde_json::json!({ "live_handoff": live_handoff }))
                    .unwrap(),
            ),
        }
    }

    /// Answers each connection with the next canned reply, recording requests.
    fn fake_server(
        path: &Path,
        replies: Vec<String>,
    ) -> std::thread::JoinHandle<Vec<serde_json::Value>> {
        let listener = UnixListener::bind(path).unwrap();
        std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                requests.push(serde_json::from_str(&request).unwrap());
                stream.write_all(reply.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
                stream.flush().unwrap();
            }
            requests
        })
    }

    fn pong(version: &str) -> String {
        format!(
            r#"{{"id":"runtime:status","result":{{"type":"pong","version":"{version}","protocol":{},"capabilities":{{"live_handoff":true}}}}}}"#,
            crate::protocol::PROTOCOL_VERSION
        )
    }

    #[test]
    fn upgrade_hands_off_to_this_binary_and_waits_for_the_new_server() {
        let path = socket_path("ok");
        let server = fake_server(
            &path,
            vec![
                r#"{"id":"upgrade:server:live-handoff","result":{"type":"ok"}}"#.into(),
                pong("7.7.7"),
            ],
        );

        let outcome = upgrade_server_at(
            &path,
            &status(true),
            Path::new("/opt/bin/herdr-benmyles"),
            "7.7.7",
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let requests = server.join().unwrap();
        let _ = fs::remove_file(&path);

        assert!(matches!(outcome, UpgradeOutcome::Upgraded));
        assert_eq!(requests[0]["method"], "server.live_handoff");
        assert_eq!(
            requests[0]["params"]["import_exe"],
            "/opt/bin/herdr-benmyles"
        );
        assert_eq!(requests[0]["params"]["expected_version"], "7.7.7");
        assert_eq!(
            requests[0]["params"]["expected_protocol"],
            crate::protocol::PROTOCOL_VERSION
        );
        assert_eq!(requests[1]["method"], "ping");
    }

    #[test]
    fn upgrade_leaves_servers_without_live_handoff_alone() {
        let path = socket_path("old");
        let outcome = upgrade_server_at(
            &path,
            &status(false),
            Path::new("/opt/bin/herdr-benmyles"),
            "7.7.7",
            Duration::from_millis(200),
            Duration::from_millis(200),
        );
        assert!(matches!(outcome, UpgradeOutcome::TooOldForHandoff));
        assert!(!path.exists(), "no request reached a server");
    }

    #[test]
    fn a_rejected_handoff_reports_the_server_still_running() {
        let path = socket_path("rejected");
        let server = fake_server(
            &path,
            vec![
                r#"{"id":"upgrade:server:live-handoff","error":{"code":"handoff_failed","message":"import server exited"}}"#.into(),
                pong("0.9.1"),
            ],
        );

        let outcome = upgrade_server_at(
            &path,
            &status(true),
            Path::new("/opt/bin/herdr-benmyles"),
            "7.7.7",
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        server.join().unwrap();
        let _ = fs::remove_file(&path);

        let UpgradeOutcome::Failed {
            error,
            still_running,
        } = outcome
        else {
            panic!("expected a failed handoff");
        };
        assert!(error.contains("import server exited"), "{error}");
        assert!(still_running);
    }
}
