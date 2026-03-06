# MRU Quick-Switch Overlay Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a centered floating modal triggered by `R` that shows the 5 most recently attached repos/sessions and lets the user attach with Enter.

**Architecture:** New `Mode::RecentPicker` variant added to the existing Mode enum. `App` stores `mru_items: Vec<(String, Option<PathBuf>)>` and `mru_selected: usize`. Rendering follows the exact same pattern as `draw_ordering_modal`. Attach logic reuses `record_opened` + `Action`.

**Tech Stack:** Rust, ratatui — no new dependencies.

---

### Task 1: Add `RecentPicker` mode and fields to App

**Files:**
- Modify: `src/app.rs:8-22` (Mode enum)
- Modify: `src/app.rs:56-99` (App struct)
- Modify: `src/app.rs:102-142` (App::new)

**Step 1: Add variant to Mode enum**

In `src/app.rs`, add `RecentPicker` to the `Mode` enum after `EditingNote`:

```rust
    EditingNote,
    RecentPicker,
```

**Step 2: Add fields to App struct**

In the `App` struct (after `ordering_selected: usize,`), add:

```rust
    pub mru_items: Vec<(String, Option<PathBuf>)>,
    pub mru_selected: usize,
```

**Step 3: Initialize in App::new**

In `App::new`, after `ordering_selected: 0,`, add:

```rust
            mru_items: Vec::new(),
            mru_selected: 0,
```

**Step 4: Build the project to confirm it compiles**

```bash
cargo build 2>&1
```
Expected: compile errors about non-exhaustive match patterns (those get fixed in later tasks). Struct/enum changes should be clean.

---

### Task 2: Add `start_recent`, `confirm_recent`, `cancel_recent` methods

**Files:**
- Modify: `src/app.rs` — add methods after `cancel_ordering` (around line 870)

**Step 1: Add `start_recent`**

After the `cancel_ordering` method, add:

```rust
    pub fn start_recent(&mut self) {
        let mut repo_paths: HashMap<String, PathBuf> = HashMap::new();
        if let Some(ref tree) = self.workspace_tree {
            collect_repo_paths(tree, &mut repo_paths);
        }

        // Constants first (sorted by name for stability), then top 5 recent non-constants.
        let mut constant_items: Vec<(String, Option<PathBuf>)> = self.constants.iter()
            .map(|name| (name.clone(), repo_paths.get(name).cloned()))
            .collect();
        constant_items.sort_by(|a, b| a.0.cmp(&b.0));

        let mut recent_entries: Vec<(&String, &u64)> = self.history.iter()
            .filter(|(name, _)| !self.constants.contains(*name))
            .collect();
        recent_entries.sort_by(|a, b| b.1.cmp(a.1));

        let recent_items: Vec<(String, Option<PathBuf>)> = recent_entries
            .into_iter()
            .take(5)
            .map(|(name, _)| (name.clone(), repo_paths.get(name).cloned()))
            .collect();

        self.mru_items = constant_items.into_iter().chain(recent_items).collect();

        if self.mru_items.is_empty() {
            return;
        }
        self.mru_selected = 0;
        self.mode = Mode::RecentPicker;
    }
```

Note: `collect_repo_paths` is a free function defined later in the file (line ~1330) — it's accessible here.

**Step 2: Add `confirm_recent`**

```rust
    pub fn confirm_recent(&mut self) {
        let (name, path) = match self.mru_items.get(self.mru_selected) {
            Some(item) => item.clone(),
            None => {
                self.mode = Mode::Normal;
                return;
            }
        };
        let session = self.all_sessions.iter().find(|s| s.name == name).cloned();
        if let Some(session) = session {
            self.record_opened(&session.name);
            self.action = Action::Attach(session.pid_name.clone());
        } else {
            self.record_opened(&name);
            self.action = Action::Create(name, path);
        }
        self.mode = Mode::Normal;
    }
```

**Step 3: Add `cancel_recent`**

```rust
    pub fn cancel_recent(&mut self) {
        self.mode = Mode::Normal;
    }
```

**Step 4: Build**

```bash
cargo build 2>&1
```
Expected: still non-exhaustive match errors in main.rs and ui.rs, but no errors in app.rs.

---

### Task 3: Wire up key handlers in main.rs

**Files:**
- Modify: `src/main.rs:281-318` (Normal mode key handler)
- Modify: `src/main.rs:434-461` (Ordering mode handler block — add RecentPicker block after it)

**Step 1: Add `R` key to Normal mode**

In the `Mode::Normal => match key.code {` block (around line 312), after `KeyCode::Char('r') => app.refresh_sessions(),`, add:

```rust
                        KeyCode::Char('R') => app.start_recent(),
```

**Step 2: Add RecentPicker key handler block**

After the `Mode::Ordering => { ... }` block (ends around line 460), add:

```rust
                    Mode::RecentPicker => match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            if app.mru_selected + 1 < app.mru_items.len() {
                                app.mru_selected += 1;
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if app.mru_selected > 0 {
                                app.mru_selected -= 1;
                            }
                        }
                        KeyCode::Enter => app.confirm_recent(),
                        KeyCode::Esc | KeyCode::Char('R') => app.cancel_recent(),
                        _ => {}
                    },
```

**Step 3: Build**

```bash
cargo build 2>&1
```
Expected: only non-exhaustive match errors in ui.rs remain.

---

### Task 4: Add `draw_recent_modal` to ui.rs

**Files:**
- Modify: `src/ui.rs:136-178` (draw fn match block)
- Modify: `src/ui.rs:1149` (add new fn after draw_ordering_modal)

**Step 1: Add arm to draw() match**

In the `match app.mode {` block inside `draw()` (around line 169), after the `Mode::EditingNote` arm, add:

```rust
        Mode::RecentPicker => {
            dim_background(f);
            draw_recent_modal(f, app);
        }
```

**Step 2: Add draw_recent_modal function**

After the closing `}` of `draw_ordering_modal` (end of file, line ~1198), add:

```rust
fn draw_recent_modal(f: &mut Frame, app: &App) {
    let area = f.area();
    let n = app.mru_items.len() as u16;
    let height = n + 2;
    let width = app.mru_items.iter()
        .map(|(name, _)| name.chars().count() as u16)
        .max()
        .unwrap_or(20)
        .max(16)
        .saturating_add(6)
        .min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            " recent ",
            Style::default().fg(MODAL_TITLE).bg(MODAL_BG).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines: Vec<Line> = app.mru_items.iter().enumerate().map(|(i, (name, _))| {
        let selected = i == app.mru_selected;
        let bg = if selected { HIGHLIGHT_BG } else { MODAL_BG };
        let prefix = if selected { " \u{2588} " } else { "   " };
        let has_session = app.all_sessions.iter().any(|s| s.name == *name);
        let fg = if has_session { GREEN } else { REPO_FG };
        Line::from(vec![
            Span::styled(prefix, Style::default().fg(ACCENT).bg(bg)),
            Span::styled(name.clone(), Style::default().fg(fg).bg(bg)),
        ])
    }).collect();

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}
```

**Step 3: Check that GREEN and REPO_FG are in scope**

```bash
grep -n "^const GREEN\|^const REPO_FG\|^pub const GREEN\|^pub const REPO_FG" src/ui.rs
```
Expected: both constants defined in ui.rs. If not, check the color constants section at top of file.

**Step 4: Build and check clean**

```bash
cargo build 2>&1
```
Expected: no errors.

---

### Task 5: Manual smoke test

**Step 1: Run the app**

```bash
cargo run 2>&1
```

**Step 2: Verify R opens the modal**

Press `R`. The modal should appear centered with up to 5 recent repo/session names. If history is empty, nothing should happen.

**Step 3: Verify navigation**

Press `j`/`k` to move selection. Highlighted row should move.

**Step 4: Verify dismiss**

Press `Esc` — modal closes, main list is unchanged. Press `R` again, then `R` again — should also close.

**Step 5: Verify attach**

Navigate to an entry with a green name (active session). Press `Enter`. Should attach to the session.
