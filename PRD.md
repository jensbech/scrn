# scrn — Product Requirements Document

> A terminal UI for managing GNU Screen sessions with workspace support,
> embedded PTY rendering, and two-pane split mode.
>
> This PRD covers bug fixes, quality improvements, and small features
> scoped to be achievable as individual atomic tasks.

---

## Phase 1 — Bug Fixes

- [ ] **TASK-01** Fix: sessions lost when navigating back from attached view in workspace mode. When a user attaches to a workspace repo session and then detaches (Esc Esc), the session should remain in the `screen -ls` list and show as active (green) in the tree. Investigate `app.rs` detach/back handling to ensure sessions are not inadvertently killed on detach. Add a status message confirming the session is still alive after detach.

- [ ] **TASK-02** Fix: companion "-2" pane sessions are visible in the session list. When two-pane mode creates a right-side companion session (name ending in `-2`), these should be hidden from the user-facing session list and workspace tree. Filter them out in `app.rs` during `rebuild_display_items()` and `refresh_sessions()` so only the primary session is shown. The companion should still be manageable internally.

- [ ] **TASK-03** Fix: config `sidebar` setting is ignored when `-w` is passed on CLI. In `config.rs`, when a CLI workspace arg is provided, the function returns early with `sidebar: false` and never reads the config file's `sidebar` value. Change `Config::load` to always read the config file for non-workspace settings (like `sidebar`), then override only the workspace field if the CLI arg is present.

- [ ] **TASK-04** Fix: search in workspace mode does not collapse empty parent directories. When using `/` to search in workspace tree mode, parent directory nodes with zero matching children should be hidden from the filtered display. Update the search/filter logic in `app.rs` to prune ancestor nodes that have no matching descendants, so the search results show a clean, minimal tree.

## Phase 2 — Code Quality & Robustness

- [ ] **TASK-05** Cap scrollback history memory. The `screen_history_left` and `screen_history_right` `Vec<String>` fields in `App` grow without bound. Add a constant `MAX_SCROLLBACK_LINES` (default 5000) and truncate from the top when appending new history lines. This prevents excessive memory use for long-running sessions.

- [ ] **TASK-06** Replace `unwrap()` calls with proper error handling. There are 5 `unwrap()` calls across `app.rs`, `screen.rs`, and `logging.rs`. Replace each with `unwrap_or_default()`, `unwrap_or_else(|| ...)`, or proper `Result` propagation as appropriate. None of these should be able to panic in normal operation.

- [ ] **TASK-07** Extract input handling from main event loop into dedicated functions. The main event loop in `main.rs` is 1300+ lines. Extract the keyboard input handling for each `Mode` (Normal, Searching, Creating, Renaming, Attached, ConfirmKill, etc.) into separate functions like `handle_normal_input(app, key) -> bool`, `handle_search_input(app, key) -> bool`, etc. Keep them in `main.rs` but as standalone functions called from the match arms.

- [ ] **TASK-08** Extract mouse event handling from main event loop into a dedicated function. Create a `handle_mouse_event(app, mouse_event, ...)` function in `main.rs` that consolidates all mouse handling logic (click, scroll, drag-resize, text selection, double-click). Replace the inline mouse handling block in the main loop with a call to this function.

## Phase 3 — UX Improvements

- [ ] **TASK-09** Persist split pane ratio to disk. When the user drags the split separator to adjust the left/right ratio, save the value to `~/.config/scrn/split_ratio` on change. Load it on startup and use it as the initial `split_left_pct` value in `App::new()`. Use a simple plain-text file containing just the integer percentage (e.g. `55`).

- [ ] **TASK-10** Persist sidebar width to disk. When the user drags the sidebar width, save it to `~/.config/scrn/sidebar_width`. Load it on startup as the initial `sidebar_width_user` value. Use a simple plain-text file containing just the integer pixel/column width.

- [ ] **TASK-11** Show a search result count in the search bar. When the user is in search/filter mode, display a count like `(3 matches)` or `(0 matches)` next to the search input in the UI. Update the count live as the user types. This gives immediate feedback on whether the filter is finding anything.

- [ ] **TASK-12** Add a visible loading indicator when refreshing sessions. When `r` is pressed to refresh or on initial load, briefly show a status message like "Refreshing sessions..." before the list updates. This provides feedback that the action was registered, especially if `screen -ls` is slow.

## Phase 4 — Small Features

- [ ] **TASK-13** Add configurable default split ratio in config.toml. Allow users to set `split_ratio = 50` (or any value 20-80) in `~/.config/scrn/config.toml`. Parse this in `config.rs` and use it as the default for `split_left_pct` when no persisted override exists. Document the option with a comment in the generated default config.

- [ ] **TASK-14** Add session sort options. Add a keybinding `s` in Normal mode that cycles through sort modes: "by name" (alphabetical), "by recent" (most recently opened first, using the history timestamps), and "by state" (attached first, then detached). Show the current sort mode in the status bar. Persist the chosen sort mode in `~/.config/scrn/sort_mode`.

- [ ] **TASK-15** Lazy session creation for workspace repos. In workspace mode, do not pre-create screen sessions for every repo. Instead, only create the session when the user presses Enter on a repo node for the first time. Show repos without sessions using a dimmed style, and repos with active sessions in the normal/green style. This avoids cluttering `screen -ls` with unused sessions.

- [ ] **TASK-16** Add `--version` and `-V` CLI flags. Print the version from `Cargo.toml` (using `env!("CARGO_PKG_VERSION")`) and exit. This is standard CLI behavior and helps users verify which version they have installed.
