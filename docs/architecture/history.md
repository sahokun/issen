# Result pinning (usage-based ranking)

Covers: `src/history.rs`, `src/app.rs`.

Previously chosen results are boosted back to the top the next time the
same query matches them.

- `src/history.rs`'s `History` tracks a `use_count` per stable key
  (`crate::search::target_key`, built from the `Action` — `Launch` uses
  only the path, never `args`, and everything but aliases uses an empty
  string, so multiple aliases pointing at the same executable are treated
  as one). `run_search` (`src/app.rs`) adds a boost
  (`HISTORY_SCORE_BOOST` = 1,000,000 + `use_count`) to each matching
  result's fuzzy score before sorting — large enough relative to fuzzy
  matching's per-character bonus (14 points max) that a pinned result
  always sorts first among results the query still matches. Results the
  query no longer matches are never force-included.
- Persisted separately from `config.toml`, at `%APPDATA%\Issen\history.toml`
  — settings are only written when the user clicks Save, but history needs
  to survive a restart without an explicit save step, so `run_result_action`
  calls `History::save` directly on every successful launch.
  - Clipboard-only results (calculator, snippets, and similar plugin
    results) are excluded from history — their value is entirely
    query-dependent and not worth pinning permanently
    (`Action::CopyToClipboard` is skipped).
- Every action a result row can trigger — running it, registering an alias,
  unpinning — is deferred to a `pending_row_action` local variable and
  processed once, *after* the results loop (`for i in 0..self.results.len()`)
  finishes, rather than inline inside the loop
  (`src/app.rs::ui`). These actions can mutate `self.results` via
  `run_result_action` (e.g. a successful launch resets the whole search
  state through `set_visible`). Since the loop bound is captured before the
  loop starts, mutating `self.results` mid-loop makes the next
  `self.results[i]` access go out of bounds — this reproduced as a real
  panic (`index out of bounds: the len is 0 but the index is 1`) when
  clicking a result to launch it. Deferring every row action to after the
  loop applies to all of them uniformly, not just the ones that happened to
  avoid resizing `self.results`.

# Query history (re-running a past search)

Separate from result pinning above — this tracks *what was searched*, not
*what was chosen*. Clicking the history icon next to the input box lists
past queries newest-first; picking one replaces the query and re-runs
search (it doesn't execute anything). Stored as `queries: Vec<String>` in
the same `History` / `history.toml` as pinning (newest first, capped at
`QUERY_HISTORY_CAP` = 20, duplicates moved to the front instead of
repeated). Recorded alongside pinning, from `run_result_action`, when the
default action succeeds and the query is non-empty.

While the history panel is open, the normal result list underneath is
stale, so `logic()` early-returns before handling result-list keyboard
shortcuts (↑/↓/Enter/Alt+N) to avoid executing an unrelated, stale result.
