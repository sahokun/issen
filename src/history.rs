use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persisted state for "pinning": boosting previously chosen (run) search
/// results back to the top. Saved to its own file (`history.toml`),
/// separate from `config.toml`. Settings-window widgets bind directly to
/// `config`, where the Save button is an explicit commit point; history
/// instead grows automatically every time a result is run, and would be
/// lost on restart if it waited for an explicit save — hence the separate
/// save path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
    /// Past search query strings, newest first. Listed from the history
    /// icon next to the input box; picking one re-runs that query. A
    /// separate field from result pinning (`entries`) because it's a
    /// different concept — this is "what was searched", pinning is "what
    /// was chosen".
    pub queries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The stable identifier returned by `crate::search::target_key`.
    pub key: String,
    pub use_count: u32,
}

/// The score boost added to a pinned result. Large enough relative to
/// `fuzzy_match`'s per-character maximum (14 points, including boundary and
/// consecutive-match bonuses) that a normal query can never outrank it.
/// Among pinned results, `use_count` decides the order (used more = ranks
/// higher).
pub const HISTORY_SCORE_BOOST: i32 = 1_000_000;

/// Cap on how many past queries are kept. Without a cap, `history.toml`
/// would grow without bound; this trims to the most recent N, the same
/// idea as `RESULT_RETENTION_CAP`.
pub const QUERY_HISTORY_CAP: usize = 20;

impl History {
    pub fn load_or_default(app_name: &str) -> Self {
        Self::history_path(app_name)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, app_name: &str) -> std::io::Result<()> {
        let path = Self::history_path(app_name).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "%APPDATA% is not set")
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        std::fs::write(path, text)
    }

    fn history_path(app_name: &str) -> Option<PathBuf> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join(app_name).join("history.toml"))
    }

    /// Called when a result is run. Increments the use count for an
    /// existing entry, or adds a new one.
    pub fn record_use(&mut self, key: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.use_count += 1;
        } else {
            self.entries.push(HistoryEntry {
                key: key.to_string(),
                use_count: 1,
            });
        }
    }

    /// Returns the score boost to add in `run_search` if `key` is pinned.
    pub fn boost_for(&self, key: &str) -> Option<i32> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| HISTORY_SCORE_BOOST + e.use_count as i32)
    }

    /// Called from the right-click menu's "unpin".
    pub fn remove(&mut self, key: &str) {
        self.entries.retain(|e| e.key != key);
    }

    /// Called from the right-click menu's "pin". Unlike `record_use`, this
    /// is an explicit user action, so it doesn't touch an existing use
    /// count accumulated through actual runs — it only adds a new entry at
    /// `use_count: 0` if one doesn't already exist (a no-op if already
    /// pinned).
    pub fn pin(&mut self, key: &str) {
        if self.entries.iter().any(|e| e.key == key) {
            return;
        }
        self.entries.push(HistoryEntry {
            key: key.to_string(),
            use_count: 0,
        });
    }

    /// Called when a search result is run. If the same query is already
    /// present, it's just moved to the front (no duplicates); anything
    /// beyond `QUERY_HISTORY_CAP` is dropped.
    pub fn record_query(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.queries.retain(|q| q != query);
        self.queries.insert(0, query.to_string());
        self.queries.truncate(QUERY_HISTORY_CAP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_use_adds_new_entry_at_count_one() {
        let mut history = History::default();
        history.record_use("a");
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].use_count, 1);
    }

    #[test]
    fn record_use_increments_existing_entry() {
        let mut history = History::default();
        history.record_use("a");
        history.record_use("a");
        history.record_use("a");
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].use_count, 3);
    }

    #[test]
    fn boost_for_is_none_when_not_pinned() {
        let history = History::default();
        assert_eq!(history.boost_for("a"), None);
    }

    #[test]
    fn boost_for_adds_use_count_on_top_of_base_boost() {
        let mut history = History::default();
        history.record_use("a");
        history.record_use("a");
        assert_eq!(history.boost_for("a"), Some(HISTORY_SCORE_BOOST + 2));
    }

    #[test]
    fn pin_adds_unrun_entry_at_count_zero() {
        let mut history = History::default();
        history.pin("a");
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].use_count, 0);
        assert_eq!(history.boost_for("a"), Some(HISTORY_SCORE_BOOST));
    }

    #[test]
    fn pin_does_not_reset_use_count_of_already_pinned_entry() {
        let mut history = History::default();
        history.record_use("a");
        history.record_use("a");
        history.pin("a");
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].use_count, 2);
    }

    #[test]
    fn remove_drops_matching_entry_only() {
        let mut history = History::default();
        history.record_use("a");
        history.record_use("b");
        history.remove("a");
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].key, "b");
    }

    #[test]
    fn remove_is_noop_when_key_absent() {
        let mut history = History::default();
        history.record_use("a");
        history.remove("does-not-exist");
        assert_eq!(history.entries.len(), 1);
    }

    #[test]
    fn record_query_moves_existing_query_to_front_without_duplicating() {
        let mut history = History::default();
        history.record_query("foo");
        history.record_query("bar");
        history.record_query("foo");
        assert_eq!(history.queries, vec!["foo", "bar"]);
    }

    #[test]
    fn record_query_ignores_blank_input() {
        let mut history = History::default();
        history.record_query("   ");
        assert!(history.queries.is_empty());
    }

    #[test]
    fn record_query_trims_before_storing() {
        let mut history = History::default();
        history.record_query("  foo  ");
        assert_eq!(history.queries, vec!["foo"]);
    }

    #[test]
    fn record_query_caps_at_query_history_cap() {
        let mut history = History::default();
        for i in 0..QUERY_HISTORY_CAP + 5 {
            history.record_query(&i.to_string());
        }
        assert_eq!(history.queries.len(), QUERY_HISTORY_CAP);
        // The most recent one stays at the front.
        assert_eq!(history.queries[0], (QUERY_HISTORY_CAP + 4).to_string());
    }
}
