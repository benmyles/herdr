//! Agent presentation shared by the agents panel and the live agent grid:
//! vendor icons and colors, status marks, idle freshness, and session titles.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::style::{Color, Modifier, Style};

use crate::api::schema::AgentStatus;
use crate::app::state::Palette;
use crate::config::{AgentIconsConfig, AgentMarksConfig};
use crate::protocol::ClientShellAgent;

/// File name prefix of the Herdr Agent Icons font faces.
const ICON_FONT_FILE_PREFIX: &str = "HerdrAgentIcons";

/// One spinner or pulse step.
const FRAME_MILLIS: u128 = 100;
/// Eight-dot braille with the gap walking round the cell.
const SPINNER: [char; 8] = ['⣷', '⣯', '⣟', '⡿', '⢿', '⣻', '⣽', '⣾'];
/// Steps each half of the blocked pulse lasts.
const PULSE_STEPS: u64 = 5;

const DONE_COLOR: Color = Color::Rgb(0x4c, 0x9a, 0x5a);
const BLOCKED_COLOR: Color = Color::Rgb(0xc0, 0x4a, 0x4a);
const UNKNOWN_COLOR: Color = Color::Rgb(0x90, 0x7a, 0xa9);
const WORKING_COLOR: Color = Color::Rgb(0xc7, 0x8a, 0x1f);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconSet {
    Font,
    Text,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentMarks {
    pub(crate) icons: IconSet,
    fresh: Duration,
    stale: Duration,
    pub(crate) animate: bool,
}

impl Default for AgentMarks {
    fn default() -> Self {
        Self::resolve(&AgentMarksConfig::default(), false)
    }
}

impl AgentMarks {
    /// Resolves `auto` icons by looking for the icon font on this machine,
    /// where the host terminal draws.
    pub(crate) fn from_config(config: &AgentMarksConfig) -> Self {
        // Tests must not depend on the fonts of the machine running them.
        let font_installed = !cfg!(test)
            && config.icons == AgentIconsConfig::Auto
            && crate::platform::font_file_installed(ICON_FONT_FILE_PREFIX);
        Self::resolve(config, font_installed)
    }

    fn resolve(config: &AgentMarksConfig, font_installed: bool) -> Self {
        let icons = match config.icons {
            AgentIconsConfig::Auto if font_installed => IconSet::Font,
            AgentIconsConfig::Auto | AgentIconsConfig::Text => IconSet::Text,
            AgentIconsConfig::Font => IconSet::Font,
            AgentIconsConfig::None => IconSet::None,
        };
        let fresh = Duration::from_secs(config.fresh_minutes.saturating_mul(60));
        let stale = Duration::from_secs(config.stale_minutes.saturating_mul(60)).max(fresh);
        Self {
            icons,
            fresh,
            stale,
            animate: config.animate,
        }
    }

    /// Presentation state for `status` given when the agent last changed.
    pub(crate) fn state(
        &self,
        status: AgentStatus,
        changed_at_ms: Option<u64>,
        now: SystemTime,
    ) -> MarkState {
        match status {
            AgentStatus::Blocked => MarkState::Blocked,
            AgentStatus::Done => MarkState::Done,
            AgentStatus::Working => MarkState::Working,
            AgentStatus::Unknown => MarkState::Unknown,
            AgentStatus::Idle => match idle_age(changed_at_ms, now) {
                Some(age) if age < self.fresh => MarkState::IdleFresh,
                Some(age) if age >= self.stale => MarkState::IdleStale,
                _ => MarkState::Idle,
            },
        }
    }

    pub(crate) fn agent_state(&self, agent: &ClientShellAgent, now: SystemTime) -> MarkState {
        self.state(agent.agent_status, agent.state_changed_at_ms, now)
    }

    /// The vendor icon for `agent`, if this set draws one.
    pub(crate) fn icon(&self, agent: &ClientShellAgent) -> Option<char> {
        let vendor = vendor(agent)?;
        match self.icons {
            IconSet::Font => Some(vendor.font),
            IconSet::Text => Some(vendor.text),
            IconSet::None => None,
        }
    }

    /// The mark drawn ahead of the title. Idle tiers have none: their title
    /// color already tells how long ago the agent last worked.
    pub(crate) fn lead(&self, state: MarkState, frame: AnimationFrame) -> Option<char> {
        let font = self.icons == IconSet::Font;
        match state {
            MarkState::Working => Some(if self.animate {
                SPINNER[(frame.0 % SPINNER.len() as u64) as usize]
            } else {
                SPINNER[0]
            }),
            MarkState::Blocked => {
                let quiet = self.animate && (frame.0 / PULSE_STEPS) % 2 == 1;
                Some(match (quiet, font) {
                    (false, true) => '\u{e1c1}',
                    (false, false) => '?',
                    (true, true) => '\u{e1c2}',
                    (true, false) => '·',
                })
            }
            MarkState::Done => Some(if font { '\u{e1c0}' } else { '✓' }),
            MarkState::Unknown => Some(if font { '\u{e1c3}' } else { '◌' }),
            MarkState::IdleFresh | MarkState::Idle | MarkState::IdleStale => None,
        }
    }
}

/// The moment a frame draws agent marks: its animation step and wall clock.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentClock {
    pub(crate) frame: AnimationFrame,
    pub(crate) now: SystemTime,
}

impl AgentClock {
    /// What a repaint must change for: the animation step while something
    /// animates, and the minute (ages and freshness tiers) always.
    pub(crate) fn repaint_key(self, animating: bool) -> (u64, u64) {
        let minute = self
            .now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs() / 60);
        (if animating { self.frame.0 } else { 0 }, minute)
    }
}

/// Animation step shared by every animated mark on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AnimationFrame(pub(crate) u64);

impl AnimationFrame {
    pub(crate) fn at(epoch: Instant, now: Instant) -> Self {
        Self((now.saturating_duration_since(epoch).as_millis() / FRAME_MILLIS) as u64)
    }
}

/// What a row shows for an agent, most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum MarkState {
    Blocked,
    Done,
    Working,
    IdleFresh,
    Idle,
    Unknown,
    IdleStale,
}

impl MarkState {
    pub(crate) fn animated(self) -> bool {
        matches!(self, Self::Working | Self::Blocked)
    }

    /// Whether the row shows how long the agent has been waiting.
    pub(crate) fn shows_age(self) -> bool {
        matches!(
            self,
            Self::Done | Self::IdleFresh | Self::Idle | Self::IdleStale
        )
    }

    pub(crate) fn title_style(self, agent: &ClientShellAgent, palette: &Palette) -> Style {
        let light = palette_is_light(palette);
        match self {
            Self::Working => Style::default()
                .fg(vendor(agent).and_then(|v| v.color).unwrap_or(WORKING_COLOR))
                .add_modifier(Modifier::BOLD),
            Self::Done => Style::default().fg(DONE_COLOR),
            Self::Blocked => Style::default().fg(BLOCKED_COLOR),
            Self::Unknown => Style::default().fg(UNKNOWN_COLOR),
            Self::IdleFresh => Style::default().fg(if light {
                Color::Rgb(0x41, 0x6c, 0x4f)
            } else {
                Color::Rgb(0x95, 0xbb, 0xa2)
            }),
            Self::Idle => Style::default().fg(if light {
                Color::Rgb(0x6b, 0x62, 0x59)
            } else {
                Color::Rgb(0xa9, 0x9e, 0x92)
            }),
            Self::IdleStale => Style::default().fg(stale_color(light)),
        }
    }

    pub(crate) fn lead_style(self, agent: &ClientShellAgent, palette: &Palette) -> Style {
        self.title_style(agent, palette)
            .remove_modifier(Modifier::BOLD)
    }

    /// Vendor icons keep their brand color unless the agent went stale.
    pub(crate) fn icon_style(self, agent: &ClientShellAgent, palette: &Palette) -> Style {
        if self == Self::IdleStale {
            return Style::default().fg(stale_color(palette_is_light(palette)));
        }
        Style::default().fg(vendor(agent).and_then(|v| v.color).unwrap_or(palette.text))
    }
}

fn stale_color(light: bool) -> Color {
    if light {
        Color::Rgb(0x69, 0x69, 0x6d)
    } else {
        Color::Rgb(0x8b, 0x8e, 0x9c)
    }
}

fn palette_is_light(palette: &Palette) -> bool {
    match palette.text {
        Color::Rgb(r, g, b) => {
            (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000 < 128
        }
        Color::Black | Color::DarkGray => true,
        _ => false,
    }
}

fn idle_age(changed_at_ms: Option<u64>, now: SystemTime) -> Option<Duration> {
    let now_ms = now.duration_since(UNIX_EPOCH).ok()?.as_millis();
    Some(Duration::from_millis(
        now_ms
            .saturating_sub(u128::from(changed_at_ms?))
            .min(u128::from(u64::MAX)) as u64,
    ))
}

/// Compact time since the agent last changed: `now`, `12m`, `3h`, `2d`.
pub(crate) fn age_label(changed_at_ms: Option<u64>, now: SystemTime) -> Option<String> {
    let minutes = idle_age(changed_at_ms, now)?.as_secs() / 60;
    Some(match minutes {
        0 => "now".into(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    })
}

struct Vendor {
    id: &'static str,
    font: char,
    text: char,
    name: &'static str,
    color: Option<Color>,
}

const fn vendor_entry(
    id: &'static str,
    font: char,
    text: char,
    name: &'static str,
    color: Option<Color>,
) -> Vendor {
    Vendor {
        id,
        font,
        text,
        name,
        color,
    }
}

const VENDORS: &[Vendor] = &[
    vendor_entry(
        "claude",
        '\u{e1a0}',
        '§',
        "Claude Code",
        Some(Color::Rgb(0xd9, 0x77, 0x57)),
    ),
    vendor_entry("codex", '\u{e1a1}', 'Λ', "Codex", None),
    vendor_entry("opencode", '\u{e1a2}', '◇', "OpenCode", None),
    vendor_entry("omp", '\u{e1a3}', 'Π', "Oh My Pi", None),
    vendor_entry(
        "cline",
        '\u{e1a4}',
        '∇',
        "Cline",
        Some(Color::Rgb(0x58, 0x68, 0x76)),
    ),
    vendor_entry("mastracode", '\u{e1a5}', '∑', "Mastra", None),
    vendor_entry(
        "kimi",
        '\u{e1a6}',
        '✨',
        "Kimi",
        Some(Color::Rgb(0x17, 0x83, 0xff)),
    ),
    vendor_entry(
        "kilo",
        '\u{e1a7}',
        '♟',
        "Kilo",
        Some(Color::Rgb(0x9a, 0x98, 0x08)),
    ),
    vendor_entry("maki", '\u{e1a8}', '✳', "Maki", None),
    vendor_entry("pi", '\u{e1a9}', 'π', "Pi", None),
    vendor_entry("hermes", '\u{e1aa}', '☪', "Hermes", None),
    vendor_entry("cursor", '\u{e1ab}', '◆', "Cursor", None),
    vendor_entry("copilot", '\u{e1ac}', '⊙', "Copilot", None),
    vendor_entry(
        "deepseek",
        '\u{e1ad}',
        '≋',
        "DeepSeek",
        Some(Color::Rgb(0x4d, 0x6b, 0xfe)),
    ),
    vendor_entry(
        "gemini",
        '\u{e1ae}',
        '✦',
        "Gemini",
        Some(Color::Rgb(0x42, 0x85, 0xf4)),
    ),
    vendor_entry("gpt", '\u{e1af}', '✺', "GPT", None),
    vendor_entry(
        "qwen",
        '\u{e1b0}',
        'Ϙ',
        "Qwen",
        Some(Color::Rgb(0x61, 0x5c, 0xed)),
    ),
    vendor_entry("grok", '\u{e1b1}', '✖', "grok", None),
    vendor_entry("agy", '\u{e1b2}', '△', "Antigravity", None),
    vendor_entry(
        "kiro",
        '\u{e1b3}',
        'Ω',
        "Kiro",
        Some(Color::Rgb(0x90, 0x46, 0xff)),
    ),
    vendor_entry("amp", '\u{e1b4}', 'Ʌ', "Amp", None),
    vendor_entry("devin", '\u{e1b5}', 'ꓓ', "Devin", None),
    vendor_entry("qodercli", '\u{e1b6}', 'Ǫ', "Qoder", None),
    vendor_entry("glm", '\u{e1b7}', 'Ƶ', "GLM", None),
];

/// The vendor a pane declares through its display label, else the detected agent.
fn vendor(agent: &ClientShellAgent) -> Option<&'static Vendor> {
    let find = |id: &str| {
        let id = id.trim();
        VENDORS
            .iter()
            .find(|vendor| vendor.id.eq_ignore_ascii_case(id))
    };
    agent
        .display_agent
        .as_deref()
        .and_then(find)
        .or_else(|| agent.agent.as_deref().and_then(find))
}

/// The agent's product name, for titles that say nothing of their own.
pub(crate) fn agent_name(agent: &ClientShellAgent) -> String {
    agent
        .name
        .clone()
        .or_else(|| vendor(agent).map(|vendor| vendor.name.to_owned()))
        .or_else(|| agent.display_agent.clone())
        .or_else(|| agent.agent.clone())
        .unwrap_or_else(|| "agent".to_owned())
}

/// The session title an agent wrote, or its name when the title only repeats
/// where the pane is (or says nothing at all).
pub(crate) fn session_title(agent: &ClientShellAgent, cwd: Option<&str>) -> String {
    let written = [
        agent.title.as_deref(),
        agent.terminal_title_stripped.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(|title| strip_attention_bracket(title.trim()).trim())
    .find(|title| !title.is_empty());
    match written {
        Some(title) if !cwd.is_some_and(|cwd| names_location(title, cwd)) => title.to_owned(),
        _ => agent_name(agent),
    }
}

/// Codex alternates `[ ! ]` and `[ . ]` in its title while it waits; the row
/// pulses its own mark for that.
fn strip_attention_bracket(title: &str) -> &str {
    let Some(rest) = title.strip_prefix('[') else {
        return title;
    };
    let Some((inside, after)) = rest.split_once(']') else {
        return title;
    };
    if matches!(inside.trim(), "!" | "." | "·") {
        after
    } else {
        title
    }
}

/// Whether `title` is only the pane's directory, as shells title panes:
/// the path, its `~` form, its last component, or `<path>: <job>`.
fn names_location(title: &str, cwd: &str) -> bool {
    let cwd = cwd.trim_end_matches('/');
    if cwd.is_empty() {
        return false;
    }
    let names_path = |text: &str| {
        text == cwd
            || text
                .strip_prefix('~')
                .is_some_and(|tail| !tail.is_empty() && cwd.ends_with(tail))
    };
    let basename = cwd.rsplit('/').next().unwrap_or(cwd);
    names_path(title)
        || title == basename
        // `<path>: <job>`, as zsh titles a running command.
        || title
            .split_once(": ")
            .is_some_and(|(head, _)| names_path(head))
        // `user@host: <path>`, the common bash prompt title.
        || title
            .rsplit_once(':')
            .is_some_and(|(head, tail)| head.contains('@') && names_path(tail.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(agent: &str, title: Option<&str>) -> ClientShellAgent {
        ClientShellAgent {
            pane_id: "p".into(),
            workspace_id: "w".into(),
            tab_id: "t".into(),
            name: None,
            display_agent: None,
            agent: Some(agent.into()),
            title: None,
            terminal_title: title.map(str::to_owned),
            terminal_title_stripped: title.map(str::to_owned),
            agent_status: AgentStatus::Idle,
            state_change_seq: 0,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
            state_changed_at_ms: None,
        }
    }

    #[test]
    fn titles_that_only_name_the_directory_fall_back_to_the_agent_name() {
        let cwd = Some("/home/me/src/notes");
        assert_eq!(session_title(&agent("codex", Some("notes")), cwd), "Codex");
        assert_eq!(
            session_title(&agent("agy", Some("~/src/notes: agy - agy")), cwd),
            "Antigravity"
        );
        assert_eq!(session_title(&agent("claude", None), cwd), "Claude Code");
        assert_eq!(
            session_title(&agent("codex", Some("[ ! ] Action Required")), cwd),
            "Action Required"
        );
        assert_eq!(
            session_title(&agent("claude", Some("notes: rewriting the parser")), cwd),
            "notes: rewriting the parser"
        );
        assert_eq!(session_title(&agent("droid", Some("  ")), cwd), "droid");
        assert_eq!(
            session_title(&agent("claude", Some("me@box: ~/src/notes")), cwd),
            "Claude Code"
        );
    }

    #[test]
    fn idle_agents_age_through_fresh_normal_and_stale() {
        let marks = AgentMarks::default();
        let now = UNIX_EPOCH + Duration::from_secs(10_000_000);
        let ago = |minutes: u64| Some((10_000_000 - minutes * 60) * 1000);
        assert_eq!(
            marks.state(AgentStatus::Idle, ago(5), now),
            MarkState::IdleFresh
        );
        assert_eq!(
            marks.state(AgentStatus::Idle, ago(30), now),
            MarkState::Idle
        );
        assert_eq!(
            marks.state(AgentStatus::Idle, ago(121), now),
            MarkState::IdleStale
        );
        assert_eq!(marks.state(AgentStatus::Idle, None, now), MarkState::Idle);
        assert_eq!(age_label(ago(0), now).as_deref(), Some("now"));
        assert_eq!(age_label(ago(42), now).as_deref(), Some("42m"));
        assert_eq!(age_label(ago(185), now).as_deref(), Some("3h"));
        assert_eq!(age_label(ago(3000), now).as_deref(), Some("2d"));
    }

    #[test]
    fn marks_follow_the_icon_set_and_animation_frame() {
        let text = AgentMarks::resolve(&AgentMarksConfig::default(), false);
        let font = AgentMarks::resolve(&AgentMarksConfig::default(), true);
        let claude = agent("claude", None);
        assert_eq!(text.icon(&claude), Some('§'));
        assert_eq!(font.icon(&claude), Some('\u{e1a0}'));
        assert_eq!(text.icon(&agent("droid", None)), None);
        assert_eq!(
            text.lead(MarkState::Working, AnimationFrame(1)),
            Some(SPINNER[1])
        );
        assert_eq!(text.lead(MarkState::Blocked, AnimationFrame(0)), Some('?'));
        assert_eq!(text.lead(MarkState::Blocked, AnimationFrame(5)), Some('·'));
        assert_eq!(text.lead(MarkState::Done, AnimationFrame(0)), Some('✓'));
        assert_eq!(text.lead(MarkState::Idle, AnimationFrame(0)), None);
        let still = AgentMarks::resolve(
            &AgentMarksConfig {
                animate: false,
                ..AgentMarksConfig::default()
            },
            false,
        );
        assert_eq!(still.lead(MarkState::Blocked, AnimationFrame(5)), Some('?'));
    }
}
