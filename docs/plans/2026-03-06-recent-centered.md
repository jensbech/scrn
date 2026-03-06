# Recent-Centered Design Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Open scrn in the recent picker automatically when ≥2 sessions have been used today, giving a fresh slate each morning.

**Architecture:** Add a today-scoped history count helper; use it to set the initial mode in `App::new()` and after each detach in `main.rs`; update `start_recent()` to show only today's history entries (no 5-item cap); color constants distinctly in the recent modal.

**Tech Stack:** Rust, ratatui, crossterm, libc (already deps)

---

### Task 1: Add `is_today` and `today_history_count` helpers

**Files:**
- Modify: `src/app.rs` (near the other free functions at bottom, around line 1361)

**Step 1: Write the failing test**

Add at the bottom of `src/app.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_history_count_excludes_constants() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut history = HashMap::new();
        history.insert("a".to_string(), now);
        history.insert("b".to_string(), now);
        history.insert("c".to_string(), now);
        let mut constants = HashSet::new();
        constants.insert("c".to_string());
        assert_eq!(today_history_count(&history, &constants), 2);
    }

    #[test]
    fn today_history_count_ignores_old_entries() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let yesterday = now - 90_000; // 25 hours ago
        let mut history = HashMap::new();
        history.insert("a".to_string(), now);
        history.insert("b".to_string(), yesterday);
        let constants = HashSet::new();
        assert_eq!(today_history_count(&history, &constants), 1);
    }
}
```

**Step 2: Run tests to verify they fail**

```bash
cd /Users/jens/proj/pers/scrn && cargo test 2>&1 | tail -20
```

Expected: compile error — `today_history_count` not found.

**Step 3: Implement helpers**

Add these two free functions to `src/app.rs`, just before `load_history()` (around line 1361):

```rust
fn is_today(ts: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as libc::time_t;
    let ts_t = ts as libc::time_t;
    unsafe {
        let now_tm = libc::localtime(&now);
        if now_tm.is_null() {
            return false;
        }
        let (now_yday, now_year) = ((*now_tm).tm_yday, (*now_tm).tm_year);
        let ts_tm = libc::localtime(&ts_t);
        if ts_tm.is_null() {
            return false;
        }
        now_yday == (*ts_tm).tm_yday && now_year == (*ts_tm).tm_year
    }
}

pub fn today_history_count(history: &HashMap<String, u64>, constants: &HashSet<String>) -> usize {
    history
        .iter()
        .filter(|(name, ts)| !constants.contains(*name) && is_today(**ts))
        .count()
}
```

**Step 4: Run tests to verify they pass**

```bash
cd /Users/jens/proj/pers/scrn && cargo test 2>&1 | tail -20
```

Expected: `test result: ok. 2 passed`

**Step 5: Commit**

```bash
cd /Users/jens/proj/pers/scrn
git add src/app.rs
git commit -m "feat: add today_history_count helper"
```

---

### Task 2: Update `start_recent()` to show today's history only

**Files:**
- Modify: `src/app.rs:888-897`

**Step 1: Replace the recent_entries block**

Find this block (lines 888–897):
```rust
        let mut recent_entries: Vec<(&String, &u64)> = self.history.iter()
            .filter(|(name, _)| !self.constants.contains(*name))
            .collect();
        recent_entries.sort_by(|a, b| b.1.cmp(a.1));

        let recent_items: Vec<(String, Option<PathBuf>)> = recent_entries
            .into_iter()
            .take(5)
            .map(|(name, _)| (name.clone(), repo_paths.get(name).cloned()))
            .collect();
```

Replace with:
```rust
        let mut recent_entries: Vec<(&String, &u64)> = self.history.iter()
            .filter(|(name, ts)| !self.constants.contains(*name) && is_today(**ts))
            .collect();
        recent_entries.sort_by(|a, b| b.1.cmp(a.1));

        let recent_items: Vec<(String, Option<PathBuf>)> = recent_entries
            .into_iter()
            .map(|(name, _)| (name.clone(), repo_paths.get(name).cloned()))
            .collect();
```

(Removed `.take(5)`, added `&& is_today(**ts)` to the filter.)

**Step 2: Build to verify**

```bash
cd /Users/jens/proj/pers/scrn && cargo build 2>&1 | tail -20
```

Expected: clean build.

**Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat: recent picker shows today's sessions only, no cap"
```

---

### Task 3: Auto-enter recent on startup

**Files:**
- Modify: `src/app.rs:106-147` (the `App::new()` function)

**Step 1: Add a `maybe_enter_recent` method**

Add this method to the `impl App` block, right after `App::new()` (after line 147):

```rust
    pub fn maybe_enter_recent(&mut self) {
        if today_history_count(&self.history, &self.constants) >= 2 {
            self.start_recent();
        }
    }
```

**Step 2: Call it in `App::new()`**

`App::new()` constructs `Self { ... }` and returns it. We can't call methods during construction, so call `maybe_enter_recent` from `main.rs` instead (covered in Task 4). Skip `App::new()` changes.

**Step 3: Build to verify**

```bash
cd /Users/jens/proj/pers/scrn && cargo build 2>&1 | tail -20
```

Expected: clean build.

**Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: add maybe_enter_recent method"
```

---

### Task 4: Call `maybe_enter_recent` on startup and after each detach

**Files:**
- Modify: `src/main.rs`

The relevant section is around line 105–108 (startup) and the loop body after `reclaim_terminal` (around line 153–158).

**Step 1: Add startup call**

Find this block (around lines 105–107):
```rust
    let mut app = App::new(cfg.workspace);
    app.refresh_sessions();
    app.restore_sessions();
```

Change to:
```rust
    let mut app = App::new(cfg.workspace);
    app.refresh_sessions();
    app.restore_sessions();
    app.maybe_enter_recent();
```

**Step 2: Add post-detach call**

Find this block (around lines 153–158):
```rust
                reclaim_terminal(&mut terminal)?;
                app.action = Action::None;
                pending_refresh = Some(app::spawn_refresh(
                    app.workspace_dir.clone(),
                    app.dir_order.clone(),
                ));
```

Change to:
```rust
                reclaim_terminal(&mut terminal)?;
                app.action = Action::None;
                app.maybe_enter_recent();
                pending_refresh = Some(app::spawn_refresh(
                    app.workspace_dir.clone(),
                    app.dir_order.clone(),
                ));
```

Do the same for `Action::Create(ref name, Some(ref dir))` and `Action::Create(ref name, None)` arms (lines ~167 and ~181) — add `app.maybe_enter_recent();` after `app.action = Action::None;` in each.

**Step 3: Build to verify**

```bash
cd /Users/jens/proj/pers/scrn && cargo build 2>&1 | tail -20
```

Expected: clean build.

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: auto-enter recent picker when >=2 sessions used today"
```

---

### Task 5: Color constants distinctly in the recent modal

**Files:**
- Modify: `src/ui.rs:1232-1242`

**Step 1: Update `draw_recent_modal` to color constants**

The `App` struct and the constants `HashSet` are accessible via the `app` parameter. Find lines 1232–1242:

```rust
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
```

Replace with:

```rust
    let lines: Vec<Line> = app.mru_items.iter().enumerate().map(|(i, (name, _))| {
        let selected = i == app.mru_selected;
        let is_const = app.constants.contains(name.as_str());
        let bg = if selected {
            HIGHLIGHT_BG
        } else if is_const {
            CONST_BG
        } else {
            MODAL_BG
        };
        let prefix = if selected { " \u{2588} " } else { "   " };
        let has_session = app.all_sessions.iter().any(|s| s.name == *name);
        let fg = if is_const {
            MATCH_FG
        } else if has_session {
            GREEN
        } else {
            REPO_FG
        };
        Line::from(vec![
            Span::styled(prefix, Style::default().fg(ACCENT).bg(bg)),
            Span::styled(name.clone(), Style::default().fg(fg).bg(bg)),
        ])
    }).collect();
```

(`CONST_BG` and `MATCH_FG` are already defined palette constants in `ui.rs`.)

**Step 2: Build to verify**

```bash
cd /Users/jens/proj/pers/scrn && cargo build 2>&1 | tail -20
```

Expected: clean build.

**Step 3: Run all tests**

```bash
cd /Users/jens/proj/pers/scrn && cargo test 2>&1 | tail -20
```

Expected: all pass.

**Step 4: Commit**

```bash
git add src/ui.rs
git commit -m "feat: color constants gold in recent picker modal"
```
