# Recent-Centered Design

## Goal

Make the recent picker the default view when it's useful, giving a fresh slate each morning.

## Behavior

On startup and after every detach, evaluate: how many distinct sessions were switched to **today** (local calendar date)? If ≥ 2, open in `RecentPicker` mode. Otherwise open in `Normal` mode.

Constants are excluded from this count. They are always shown at the top of the recent picker in gold, but don't influence the gate.

## Changes

### `app.rs`

- Add `fn today_history_count(history: &HashMap<String, u64>) -> usize` — counts history entries whose unix timestamp falls on today's local date.
- Add `fn should_start_recent(history: &HashMap<String, u64>, constants: &HashSet<String>) -> bool` — returns `today_history_count > = 2` (constants excluded).
- In `App::new()`, after loading history, set `mode = RecentPicker` if `should_start_recent`.
- In `start_recent()`, change the recent entries to: today's history entries (sorted most-recent-first, no cap), instead of all-time last 5. Constants remain at top.

### `ui.rs`

- In `draw_recent_modal`, render constants with `CONST_*` gold background/color (matching main list treatment) to visually distinguish them from today-history entries.

### `main.rs`

- After `reclaim_terminal()` in the attach loop, call `app.maybe_enter_recent()` (or inline the check) to re-evaluate and set mode before the next `run_picker` call.

## Non-changes

- Escape from recent still falls back to `Normal`.
- The `R` keybinding still manually opens recent from Normal mode.
- The 5-item cap is removed for the today-scoped list (natural cap since only today's sessions appear).
