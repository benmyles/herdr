//! How often and how recently each command palette item was chosen, kept in
//! the client state directory so the palette ranks habits first. Uses decay
//! with a two-week half-life, so a burst of old picks fades behind current
//! ones.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 60.0 * 60.0;
/// Items kept on disk; the least used are dropped past this.
const MAX_ITEMS: usize = 300;
/// Largest boost, a little under a strong title match, so habits reorder
/// close matches without burying a clearly better one.
const MAX_BOOST: f64 = 90.0;

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub(super) struct PaletteUsage {
    #[serde(default)]
    items: HashMap<String, ItemUse>,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
struct ItemUse {
    /// Decayed pick count as of `last_used`.
    weight: f64,
    /// Unix seconds of the latest pick.
    last_used: u64,
}

impl ItemUse {
    fn weight_at(self, now: u64) -> f64 {
        let age = now.saturating_sub(self.last_used) as f64;
        self.weight * 0.5f64.powf(age / HALF_LIFE_SECS)
    }
}

impl PaletteUsage {
    pub(super) fn record(&mut self, key: &str, now: u64) {
        let weight = self.items.get(key).map_or(0.0, |item| item.weight_at(now));
        self.items.insert(
            key.to_owned(),
            ItemUse {
                weight: weight + 1.0,
                last_used: now,
            },
        );
        if self.items.len() > MAX_ITEMS {
            let mut weights = self
                .items
                .iter()
                .map(|(key, item)| (item.weight_at(now), key.clone()))
                .collect::<Vec<_>>();
            weights.sort_by(|left, right| left.0.total_cmp(&right.0));
            for (_, key) in weights.into_iter().take(self.items.len() - MAX_ITEMS) {
                self.items.remove(&key);
            }
        }
    }

    /// Ranking points for `key`: zero when never chosen, rising quickly for
    /// the first picks and levelling off toward `MAX_BOOST`.
    pub(super) fn boost(&self, key: &str, now: u64) -> i32 {
        self.items.get(key).map_or(0, |item| {
            (item.weight_at(now).ln_1p() * 30.0).min(MAX_BOOST).round() as i32
        })
    }
}

pub(super) fn path() -> PathBuf {
    crate::config::state_dir()
        .join("client-shell")
        .join("command-palette.json")
}

pub(super) fn load(path: &Path) -> PaletteUsage {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub(super) fn store(path: &Path, usage: &PaletteUsage) -> Result<(), String> {
    super::preferences::write_json_atomically(path, usage)
}

pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn picks_raise_the_boost_and_it_levels_off() {
        let mut usage = PaletteUsage::default();
        assert_eq!(usage.boost("command:new tab", 0), 0);
        usage.record("command:new tab", 0);
        let once = usage.boost("command:new tab", 0);
        usage.record("command:new tab", 0);
        usage.record("command:new tab", 0);
        let thrice = usage.boost("command:new tab", 0);
        assert!(0 < once && once < thrice, "{once} {thrice}");
        for _ in 0..200 {
            usage.record("command:new tab", 0);
        }
        assert_eq!(usage.boost("command:new tab", 0), MAX_BOOST as i32);
        assert_eq!(usage.boost("command:close tab", 0), 0);
    }

    #[test]
    fn old_picks_fade_behind_recent_ones() {
        let mut usage = PaletteUsage::default();
        for _ in 0..4 {
            usage.record("old", 0);
        }
        usage.record("recent", 60 * DAY);
        usage.record("recent", 60 * DAY);
        let now = 60 * DAY;
        assert!(usage.boost("recent", now) > usage.boost("old", now));
        assert!(usage.boost("old", now) < usage.boost("old", 0));
    }

    #[test]
    fn the_least_used_items_are_dropped_past_the_cap() {
        let mut usage = PaletteUsage::default();
        usage.record("favorite", 0);
        usage.record("favorite", 0);
        for index in 0..MAX_ITEMS {
            usage.record(&format!("item {index}"), 0);
        }
        assert_eq!(usage.items.len(), MAX_ITEMS);
        assert!(usage.boost("favorite", 0) > 0, "the most used item stays");
    }

    #[test]
    fn usage_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-palette-usage-{}-{}",
            std::process::id(),
            now()
        ));
        let path = dir.join("command-palette.json");
        assert_eq!(
            load(&path),
            PaletteUsage::default(),
            "a missing file is empty"
        );

        let mut usage = PaletteUsage::default();
        usage.record("settings:theme", 5);
        store(&path, &usage).unwrap();
        assert_eq!(load(&path), usage);

        std::fs::write(&path, "not json").unwrap();
        assert_eq!(
            load(&path),
            PaletteUsage::default(),
            "a corrupt file is empty"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
