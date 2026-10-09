//! Opening a directory as a project, in a new window, with a desktop code
//! editor: one Herdr knows and finds on PATH, or a command the user sets.

use std::path::Path;
use std::process::Stdio;

use crate::config::{EditorAppConfig, EditorConfig};

/// An editor Herdr knows how to open a project in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KnownEditor {
    VsCode,
    Zed,
    Cursor,
}

impl KnownEditor {
    /// Every known editor, in the order `auto` tries them.
    pub(crate) const ALL: [Self; 3] = [Self::VsCode, Self::Zed, Self::Cursor];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code",
            Self::Zed => "Zed",
            Self::Cursor => "Cursor",
        }
    }

    /// The command line tool the editor puts on PATH.
    pub(crate) fn program(self) -> &'static str {
        match self {
            Self::VsCode => "code",
            Self::Zed => "zed",
            Self::Cursor => "cursor",
        }
    }

    /// Arguments that open a directory as a project in a new window.
    fn new_window_args(self) -> &'static [&'static str] {
        match self {
            Self::VsCode | Self::Cursor => &["--new-window"],
            Self::Zed => &["--new"],
        }
    }

    pub(crate) fn app(self) -> EditorAppConfig {
        match self {
            Self::VsCode => EditorAppConfig::Vscode,
            Self::Zed => EditorAppConfig::Zed,
            Self::Cursor => EditorAppConfig::Cursor,
        }
    }

    fn from_app(app: EditorAppConfig) -> Option<Self> {
        Self::ALL.into_iter().find(|editor| editor.app() == app)
    }
}

/// The known editors whose command line tools are on PATH.
pub(crate) fn installed_editors() -> Vec<KnownEditor> {
    KnownEditor::ALL
        .into_iter()
        .filter(|editor| crate::integration::command_available(editor.program()))
        .collect()
}

/// The editor `auto` uses: the first known editor in `installed`.
pub(crate) fn auto_editor(installed: &[KnownEditor]) -> Option<KnownEditor> {
    KnownEditor::ALL
        .into_iter()
        .find(|editor| installed.contains(editor))
}

/// A command that opens a directory added as its last argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

/// The command for the editor `config` chooses, with `installed` the known
/// editors on PATH. `None` when `auto` finds none, or the custom command is
/// empty or has an unmatched quote.
pub(crate) fn editor_command(
    config: &EditorConfig,
    installed: &[KnownEditor],
) -> Option<EditorCommand> {
    let editor = match config.app {
        EditorAppConfig::Auto => auto_editor(installed)?,
        EditorAppConfig::Custom => return custom_command(&config.command),
        app => KnownEditor::from_app(app)?,
    };
    Some(EditorCommand {
        program: editor.program().to_owned(),
        args: editor
            .new_window_args()
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect(),
    })
}

/// Splits a custom command the way a shell would, without running one.
pub(crate) fn custom_command(command: &str) -> Option<EditorCommand> {
    let mut words = shell_words::split(command).ok()?.into_iter();
    let program = words.next()?;
    Some(EditorCommand {
        program,
        args: words.collect(),
    })
}

/// Starts `command` on `directory`, run from that directory with no
/// terminal: editor command line tools hand the project to the app and exit.
pub(crate) fn spawn(
    command: &EditorCommand,
    directory: &Path,
) -> std::io::Result<std::process::Child> {
    let directory_arg = directory.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the directory path is not valid UTF-8",
        )
    })?;
    let mut args = command.args.clone();
    args.push(directory_arg.to_owned());
    let mut process =
        crate::plugin_command::command_for_argv_in_dir(&command.program, &args, directory);
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    process.spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(app: EditorAppConfig, command: &str) -> EditorConfig {
        EditorConfig {
            app,
            command: command.to_owned(),
        }
    }

    fn command(program: &str, args: &[&str]) -> Option<EditorCommand> {
        Some(EditorCommand {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        })
    }

    #[test]
    fn auto_prefers_vs_code_then_zed_then_cursor_on_path() {
        let auto = config(EditorAppConfig::Auto, "");
        let all = KnownEditor::ALL;
        assert_eq!(
            editor_command(&auto, &all),
            command("code", &["--new-window"])
        );
        assert_eq!(
            editor_command(&auto, &[KnownEditor::Cursor, KnownEditor::Zed]),
            command("zed", &["--new"])
        );
        assert_eq!(
            editor_command(&auto, &[KnownEditor::Cursor]),
            command("cursor", &["--new-window"])
        );
        assert_eq!(editor_command(&auto, &[]), None);
    }

    #[test]
    fn a_chosen_editor_opens_in_a_new_window_whether_or_not_it_was_found() {
        assert_eq!(
            editor_command(&config(EditorAppConfig::Zed, ""), &[]),
            command("zed", &["--new"])
        );
        assert_eq!(
            editor_command(&config(EditorAppConfig::Cursor, "ignored"), &[]),
            command("cursor", &["--new-window"])
        );
    }

    #[test]
    fn custom_commands_split_like_a_shell_command_line() {
        let custom = |text| editor_command(&config(EditorAppConfig::Custom, text), &[]);
        assert_eq!(
            custom(r#"subl --new-window"#),
            command("subl", &["--new-window"])
        );
        assert_eq!(
            custom(r#"'/Applications/My Editor.app/bin/edit' -p "work space""#),
            command(
                "/Applications/My Editor.app/bin/edit",
                &["-p", "work space"]
            )
        );
        assert_eq!(custom("  "), None, "no command set");
        assert_eq!(custom("code \"unclosed"), None);
    }

    #[cfg(unix)]
    #[test]
    fn spawn_runs_the_command_from_the_directory_with_it_as_the_last_argument() {
        let directory = std::env::temp_dir().join(format!("herdr-editor-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let out = directory.join("out");
        let script = format!(
            "printf '%s|%s|%s' \"$PWD\" \"$1\" \"$2\" > '{}'",
            out.display()
        );
        let command = EditorCommand {
            program: "sh".into(),
            args: vec!["-c".into(), script, "editor".into(), "--new".into()],
        };
        let status = spawn(&command, &directory).unwrap().wait().unwrap();
        assert!(status.success());
        let written = std::fs::read_to_string(&out).unwrap();
        let canonical = directory.canonicalize().unwrap();
        let [pwd, flag, path] = written.split('|').collect::<Vec<_>>()[..] else {
            panic!("{written}");
        };
        assert_eq!(std::path::Path::new(pwd).canonicalize().unwrap(), canonical);
        assert_eq!(flag, "--new");
        assert_eq!(path, directory.to_str().unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
