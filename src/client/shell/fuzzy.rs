//! Fuzzy matching for the command palette. A query matches text when its
//! characters appear in order, ignoring case; runs of consecutive characters
//! and characters that start words score higher, and gaps cost a little.

const MATCH: i32 = 16;
const CONSECUTIVE: i32 = 16;
const TEXT_START: i32 = 14;
const WORD_START: i32 = 10;
const CAMEL_HUMP: i32 = 6;
const GAP_START: i32 = 3;
/// Extra cost when a gap lands inside a word rather than at its start.
const MID_WORD_GAP: i32 = 6;
const GAP: i32 = 1;
const MAX_LEADING_PENALTY: i32 = 12;
const NONE: i32 = i32::MIN / 2;

/// Score of `query` against `text` and the char indices of `text` it matched,
/// or `None` when the query's characters do not all appear in order. The best
/// placement wins, so "grid" in "go right in dev · agent grid" matches the
/// word, not the letters scattered before it.
pub(super) fn fuzzy_match(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let query = query
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let original = text.chars().collect::<Vec<_>>();
    // Lowercasing one char into several would shift indices; keep the first.
    let lowered = original
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect::<Vec<_>>();
    let (m, n) = (query.len(), lowered.len());
    if m > n {
        return None;
    }
    let bonus = |j: usize| -> i32 {
        if j == 0 {
            return TEXT_START;
        }
        let (previous, current) = (original[j - 1], original[j]);
        if !previous.is_alphanumeric() {
            WORD_START
        } else if previous.is_lowercase() && current.is_uppercase() {
            CAMEL_HUMP
        } else {
            0
        }
    };

    // score[i * n + j]: best total with query[i] matched at text[j].
    let mut score = vec![NONE; m * n];
    let mut from = vec![usize::MAX; m * n];
    for j in 0..n {
        if lowered[j] == query[0] {
            score[j] = MATCH + bonus(j) - (j as i32).min(MAX_LEADING_PENALTY);
        }
    }
    for (i, &wanted) in query.iter().enumerate().skip(1) {
        let (row, previous_row) = (i * n, (i - 1) * n);
        // Best `score[i - 1][k] + k * GAP` over k <= j - 2, for gapped moves.
        let mut best_gap = (NONE, usize::MAX);
        for j in i..n {
            if j >= 2 {
                let k = j - 2;
                let candidate = score[previous_row + k].saturating_add(k as i32 * GAP);
                if score[previous_row + k] > NONE && candidate > best_gap.0 {
                    best_gap = (candidate, k);
                }
            }
            if lowered[j] != wanted {
                continue;
            }
            let consecutive = score[previous_row + j - 1];
            let consecutive = (consecutive > NONE).then(|| (consecutive + CONSECUTIVE, j - 1));
            let gapped = (best_gap.0 > NONE)
                .then(|| (best_gap.0 - (j as i32 - 1) * GAP - GAP_START, best_gap.1));
            let Some((total, k)) = [consecutive, gapped]
                .into_iter()
                .flatten()
                .max_by_key(|c| c.0)
            else {
                continue;
            };
            let landing = bonus(j);
            let mid_word = if k + 1 < j && landing == 0 {
                MID_WORD_GAP
            } else {
                0
            };
            score[row + j] = total + MATCH + landing - mid_word;
            from[row + j] = k;
        }
    }

    let last = (m - 1) * n;
    let (end, best) = (0..n)
        .map(|j| (j, score[last + j]))
        .filter(|(_, s)| *s > NONE)
        .max_by_key(|(j, s)| (*s, std::cmp::Reverse(*j)))?;
    let mut positions = vec![0; m];
    let mut j = end;
    for i in (0..m).rev() {
        positions[i] = j;
        j = from[i * n + j];
    }
    Some((best, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, text: &str) -> i32 {
        fuzzy_match(query, text).map_or(NONE, |(score, _)| score)
    }

    #[test]
    fn matches_characters_in_order_ignoring_case() {
        assert_eq!(fuzzy_match("nt", "New Tab").unwrap().1, vec![0, 4]);
        assert!(fuzzy_match("tn", "new tab").is_none());
        assert!(fuzzy_match("xyz", "new tab").is_none());
        assert_eq!(fuzzy_match("", "anything").unwrap(), (0, Vec::new()));
    }

    #[test]
    fn prefers_consecutive_and_word_start_placements() {
        // "grid" matches the word, not letters scattered before it.
        assert_eq!(
            fuzzy_match("grid", "go right in dev · agent grid")
                .unwrap()
                .1,
            vec![24, 25, 26, 27]
        );
        assert!(score("tab", "new tab") > score("tab", "the atlas board"));
        assert!(score("st", "split vertical") < score("st", "settings theme"));
    }

    #[test]
    fn earlier_and_tighter_matches_rank_higher() {
        assert!(score("zoom", "zoom pane") > score("zoom", "toggle zoom pane"));
        assert!(score("cl", "close pane") > score("cl", "cycle pane left"));
        assert!(score("claude", "claude") > score("claude", "c l a u d e"));
    }

    #[test]
    fn camel_humps_count_as_word_starts() {
        assert_eq!(fuzzy_match("ab", "AgentBar").unwrap().1, vec![0, 5]);
        assert!(score("ab", "AgentBar") > score("ab", "Agentbar"));
    }
}
