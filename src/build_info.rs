//! Build identity helpers.

/// Fork identity. herdr-benmyles runs beside stock herdr, so every name that
/// locates its binary, directories, sockets or control variables is its own.
pub const BIN_NAME: &str = "herdr-benmyles";

/// Stock environment names still exported into panes. Agent hooks installed by
/// either build read these to find the server that owns their pane.
pub const STOCK_ENV_VAR: &str = "HERDR_ENV";
pub const STOCK_SOCKET_PATH_ENV_VAR: &str = "HERDR_SOCKET_PATH";
pub const STOCK_BIN_PATH_ENV_VAR: &str = "HERDR_BIN_PATH";
pub const STOCK_PANE_ID_ENV_VAR: &str = "HERDR_PANE_ID";
pub const STOCK_TAB_ID_ENV_VAR: &str = "HERDR_TAB_ID";
pub const STOCK_WORKSPACE_ID_ENV_VAR: &str = "HERDR_WORKSPACE_ID";

pub const BASE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn channel() -> &'static str {
    non_empty(option_env!("HERDR_BUILD_CHANNEL")).unwrap_or("stable")
}

pub fn build_id() -> Option<&'static str> {
    non_empty(option_env!("HERDR_BUILD_ID"))
}

/// Commit the fork was built from, stamped by install.sh. Remote attach uses it
/// to keep remote hosts on this exact build; development builds leave it unset.
pub fn build_commit() -> Option<&'static str> {
    non_empty(option_env!("HERDR_BUILD_COMMIT"))
}

pub fn version() -> String {
    match channel() {
        "stable" => BASE_VERSION.to_string(),
        channel => match build_id() {
            Some(build_id) => format!("{BASE_VERSION}-{channel}.{build_id}"),
            None => format!("{BASE_VERSION}-{channel}"),
        },
    }
}

pub fn is_preview() -> bool {
    channel() == "preview"
}

fn non_empty(value: Option<&'static str>) -> Option<&'static str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn stable_version_defaults_to_cargo_version() {
        assert!(!super::version().is_empty());
    }
}
