use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::pty::PtySession;
use crate::screen::{self, Session};
use crate::workspace::{self, TreeNode};

#[derive(PartialEq)]
pub enum Mode {
    Normal,
    Searching,
    Creating,
    Renaming,
    ConfirmKill,
    ConfirmKillAll1,
    ConfirmKillAll2,
    ConfirmQuit,
    Attached,
}

pub enum Action {
    GoHome,
    None,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Pane {
    Left,
    Right,
}

#[derive(PartialEq, Clone, Copy)]
pub enum SidebarFocus {
    List,
    Content,
}

#[derive(Clone)]
pub enum ListItem {
    SectionHeader(String),
    SessionItem(Session),
    TreeDir {
        name: String,
        prefix: String,
    },
    TreeRepo {
        name: String,
        path: PathBuf,
        session: Option<Session>,
        prefix: String,
    },
}

pub struct App {
    pub sessions: Vec<Session>,
    pub all_sessions: Vec<Session>,
    pub selected: usize,
    pub mode: Mode,
    pub search_input: String,
    pub create_input: String,
    pub cursor_pos: usize,
    pub status_msg: String,
    pub status_set_at: Instant,
    pub current_session: Option<String>,
    pub action_file: Option<String>,
    pub action: Action,
    pub show_legend: bool,
    pub pty_session: Option<PtySession>,
    pub pty_right: Option<PtySession>,
    pub active_pane: Pane,
    pub attached_name: String,
    pub attached_right_name: String,
    pub rename_pid_name: String,
    pub workspace_dir: Option<PathBuf>,
    pub workspace_tree: Option<TreeNode>,
    pub display_items: Vec<ListItem>,
    pub selectable_indices: Vec<usize>,
    pub kill_session_info: Option<(String, String)>,
    /// Pre-loaded Screen scrollback history (plain text, no colors)
    pub screen_history_left: Vec<String>,
    pub screen_history_right: Vec<String>,
    /// session name -> unix timestamp of last attach
    pub history: HashMap<String, u64>,
    pub filter_opened: bool,
    /// pinned session/repo names — always shown at top
    pub pins: HashSet<String>,
    pub sidebar_mode: bool,
    pub sidebar_focus: SidebarFocus,
    /// Table scroll offset from last render, used for click-to-select.
    pub sidebar_table_offset: std::cell::Cell<usize>,
}

impl App {
    pub fn new(action_file: Option<String>, workspace: Option<PathBuf>, sidebar_mode: bool) -> Self {
        Self {
            sessions: Vec::new(),
            all_sessions: Vec::new(),
            selected: 0,
            mode: Mode::Normal,
            search_input: String::new(),
            create_input: String::new(),
            cursor_pos: 0,
            status_msg: String::new(),
            status_set_at: Instant::now(),
            current_session: std::env::var("STY").ok(),
            action_file,
            action: Action::None,
            show_legend: false,
            pty_session: None,
            pty_right: None,
            active_pane: Pane::Left,
            attached_name: String::new(),
            attached_right_name: String::new(),
            rename_pid_name: String::new(),
            workspace_dir: workspace,
            workspace_tree: None,
            display_items: Vec::new(),
            selectable_indices: Vec::new(),
            kill_session_info: None,
            screen_history_left: Vec::new(),
            screen_history_right: Vec::new(),
            history: load_history(),
            filter_opened: false,
            pins: load_pins(),
            sidebar_mode,
            sidebar_focus: SidebarFocus::List,
            sidebar_table_offset: std::cell::Cell::new(0),
        }
    }

    pub fn refresh_sessions(&mut self) {
        match screen::list_sessions() {
            Ok(sessions) => {
                self.all_sessions = sessions;
            }
            Err(e) => {
                self.set_status(format!("Error: {e}"));
            }
        }
        if let Some(ref dir) = self.workspace_dir {
            self.workspace_tree = Some(workspace::scan_tree(dir));
        }
        save_sessions(&self.all_sessions, &self.workspace_tree);
        self.apply_search_filter();
    }

    pub fn restore_sessions(&mut self) {
        let saved = load_saved_sessions();
        if saved.is_empty() {
            return;
        }

        let live_names: std::collections::HashSet<String> =
            self.all_sessions.iter().map(|s| s.name.clone()).collect();

        let mut restored = 0;
        for (name, path) in &saved {
            if live_names.contains(name) {
                continue;
            }
            let result = if let Some(dir) = path {
                screen::create_session_in_dir(name, dir)
            } else {
                screen::create_session(name)
            };
            match result {
                Ok(()) => restored += 1,
                Err(e) => crate::logging::log_error(&format!("Failed to restore session '{name}': {e}")),
            }
        }

        if restored > 0 {
            self.refresh_sessions();
            self.set_status(format!(
                "Restored {restored} session{}",
                if restored == 1 { "" } else { "s" }
            ));
        }
    }

    pub fn set_status(&mut self, msg: String) {
        if msg.starts_with("Error") || msg.starts_with("Failed") {
            crate::logging::log_error(&msg);
        }
        self.status_msg = msg;
        self.status_set_at = Instant::now();
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if !self.selectable_indices.is_empty() && self.selected < self.selectable_indices.len() - 1 {
            self.selected += 1;
        }
    }

    pub fn move_to_top(&mut self) {
        self.selected = 0;
    }

    pub fn move_to_bottom(&mut self) {
        if !self.selectable_indices.is_empty() {
            self.selected = self.selectable_indices.len() - 1;
        }
    }

    pub fn toggle_opened_filter(&mut self) {
        self.filter_opened = !self.filter_opened;
        self.rebuild_display_list();
    }

    pub fn toggle_pin(&mut self) {
        let name = match self.selected_display_item() {
            Some(ListItem::TreeRepo { name, .. }) => name.clone(),
            Some(ListItem::SessionItem(session)) => session.name.clone(),
            _ => return,
        };
        if self.pins.contains(&name) {
            self.pins.remove(&name);
            self.set_status(format!("Unpinned '{name}'"));
        } else {
            self.pins.insert(name.clone());
            self.set_status(format!("Pinned '{name}'"));
        }
        save_pins(&self.pins);
        self.rebuild_display_list();
    }

    pub fn start_search(&mut self) {
        self.mode = Mode::Searching;
        self.search_input.clear();
    }

    pub fn apply_search_filter(&mut self) {
        self.sessions = self
            .all_sessions
            .iter()
            .filter(|s| {
                if self.is_current_session(s) {
                    return false;
                }
                if s.name.starts_with("tty") || s.name.starts_with("pts") {
                    return false;
                }
                if self.search_input.is_empty() {
                    true
                } else {
                    let haystack = format!("{} {}", s.name, s.pid_name);
                    fuzzy_match(&haystack, &self.search_input).is_some()
                }
            })
            .cloned()
            .collect();
        self.rebuild_display_list();
    }

    fn rebuild_display_list(&mut self) {
        self.display_items.clear();
        self.selectable_indices.clear();

        // Clone sessions to avoid borrow conflicts
        let sessions_clone: Vec<Session> = self.sessions.clone();
        let session_map: std::collections::HashMap<&str, &Session> =
            sessions_clone.iter().map(|s| (s.name.as_str(), s)).collect();

        let mut merged_sessions: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Build workspace items
        let mut ws_items: Vec<ListItem> = Vec::new();
        let mut ws_selectable: Vec<usize> = Vec::new();

        if let Some(ref tree) = self.workspace_tree {
            let tree = tree.clone();
            if self.search_input.is_empty() {
                flatten_tree(
                    &tree,
                    0,
                    &session_map,
                    &mut merged_sessions,
                    &mut ws_items,
                    &mut ws_selectable,
                    &mut Vec::new(),
                );
            } else {
                let query = self.search_input.clone();
                flatten_filtered(
                    &tree,
                    0,
                    &query,
                    &session_map,
                    &mut merged_sessions,
                    &mut ws_items,
                    &mut ws_selectable,
                    &mut Vec::new(),
                );
            }
        }

        // Orphan sessions: not merged into any tree repo, and not a *-2 pane session
        let mut orphan_sessions: Vec<&Session> = sessions_clone
            .iter()
            .filter(|s| {
                !merged_sessions.contains(&s.name)
                    && !s.name.ends_with("-2")
            })
            .collect();

        // Sort by fuzzy match score when searching
        if !self.search_input.is_empty() {
            orphan_sessions.sort_by(|a, b| {
                let score_a = fuzzy_match(&a.name, &self.search_input)
                    .map(|(_, s)| s)
                    .unwrap_or(i32::MIN);
                let score_b = fuzzy_match(&b.name, &self.search_input)
                    .map(|(_, s)| s)
                    .unwrap_or(i32::MIN);
                score_b.cmp(&score_a)
            });
        }

        // Build orphan items
        let mut orphan_items: Vec<ListItem> = Vec::new();
        let mut orphan_selectable: Vec<usize> = Vec::new();

        if !orphan_sessions.is_empty() {
            orphan_items.push(ListItem::SectionHeader("Sessions".to_string()));
            for session in &orphan_sessions {
                let idx = orphan_items.len();
                orphan_items.push(ListItem::SessionItem((*session).clone()));
                orphan_selectable.push(idx);
            }
        }

        // Extract pinned items from workspace and orphan groups
        let pins = &self.pins;
        let mut pinned_items: Vec<ListItem> = Vec::new();

        // Extract pinned TreeRepo items from ws_items (flatten prefix for pinned section)
        let mut pinned_ws_indices: HashSet<usize> = HashSet::new();
        for (i, item) in ws_items.iter().enumerate() {
            if let ListItem::TreeRepo { name, .. } = item {
                if pins.contains(name.as_str()) {
                    pinned_ws_indices.insert(i);
                    if let ListItem::TreeRepo { name, path, session, .. } = item {
                        pinned_items.push(ListItem::TreeRepo {
                            name: name.clone(),
                            path: path.clone(),
                            session: session.clone(),
                            prefix: String::new(),
                        });
                    }
                }
            }
        }

        // Extract pinned SessionItem items from orphan_items
        let mut pinned_orphan_indices: HashSet<usize> = HashSet::new();
        for (i, item) in orphan_items.iter().enumerate() {
            if let ListItem::SessionItem(session) = item {
                if pins.contains(session.name.as_str()) {
                    pinned_orphan_indices.insert(i);
                    pinned_items.push(item.clone());
                }
            }
        }

        // Remove pinned items from their original groups
        if !pinned_ws_indices.is_empty() {
            let old_ws = std::mem::take(&mut ws_items);
            let old_sel = std::mem::take(&mut ws_selectable);
            let sel_set: HashSet<usize> = old_sel.into_iter().collect();
            for (i, item) in old_ws.into_iter().enumerate() {
                if pinned_ws_indices.contains(&i) {
                    continue;
                }
                let new_idx = ws_items.len();
                if sel_set.contains(&i) {
                    ws_selectable.push(new_idx);
                }
                ws_items.push(item);
            }
        }
        if !pinned_orphan_indices.is_empty() {
            let old_orphan = std::mem::take(&mut orphan_items);
            let old_sel = std::mem::take(&mut orphan_selectable);
            let sel_set: HashSet<usize> = old_sel.into_iter().collect();
            for (i, item) in old_orphan.into_iter().enumerate() {
                if pinned_orphan_indices.contains(&i) {
                    continue;
                }
                let new_idx = orphan_items.len();
                if sel_set.contains(&i) {
                    orphan_selectable.push(new_idx);
                }
                orphan_items.push(item);
            }
        }

        // Prune orphan section if all items were pinned (only header remains)
        if orphan_selectable.is_empty() {
            orphan_items.clear();
        }

        // Append pinned section first
        if !pinned_items.is_empty() {
            self.display_items.push(ListItem::SectionHeader("Pinned".to_string()));
            for item in pinned_items {
                let idx = self.display_items.len();
                self.selectable_indices.push(idx);
                self.display_items.push(item);
            }
        }

        // When searching, hoist the group with the best match score first
        let orphans_first = if !self.search_input.is_empty() {
            let best_ws_score = ws_items.iter().filter_map(|item| {
                if let ListItem::TreeRepo { name, .. } = item {
                    fuzzy_match(name, &self.search_input).map(|(_, s)| s)
                } else {
                    None
                }
            }).max().unwrap_or(i32::MIN);

            let best_orphan_score = orphan_sessions.iter().filter_map(|s| {
                fuzzy_match(&s.name, &self.search_input).map(|(_, sc)| sc)
            }).max().unwrap_or(i32::MIN);

            best_orphan_score > best_ws_score
        } else {
            false
        };

        // Helper: append a group's items, adjusting selectable indices to the current offset
        let append_group = |items: Vec<ListItem>, sel: Vec<usize>, dest: &mut Vec<ListItem>, dest_sel: &mut Vec<usize>| {
            let offset = dest.len();
            for idx in sel {
                dest_sel.push(offset + idx);
            }
            dest.extend(items);
        };

        if orphans_first {
            append_group(orphan_items, orphan_selectable, &mut self.display_items, &mut self.selectable_indices);
            append_group(ws_items, ws_selectable, &mut self.display_items, &mut self.selectable_indices);
        } else {
            append_group(ws_items, ws_selectable, &mut self.display_items, &mut self.selectable_indices);
            append_group(orphan_items, orphan_selectable, &mut self.display_items, &mut self.selectable_indices);
        }

        // Apply "opened only" filter
        if self.filter_opened {
            let history = &self.history;
            let mut filtered_items = Vec::new();
            let mut filtered_indices = Vec::new();
            for (i, item) in self.display_items.iter().enumerate() {
                match item {
                    ListItem::TreeRepo { name, session, .. } => {
                        if session.is_some() && history.contains_key(name.as_str()) {
                            filtered_indices.push(filtered_items.len());
                            filtered_items.push(item.clone());
                        }
                    }
                    ListItem::SessionItem(session) => {
                        if history.contains_key(session.name.as_str()) {
                            filtered_indices.push(filtered_items.len());
                            filtered_items.push(item.clone());
                        }
                    }
                    ListItem::TreeDir { .. } | ListItem::SectionHeader(_) => {
                        // Keep dirs/headers only if selectable items follow;
                        // we'll prune empty ones in a second pass
                        if self.selectable_indices.contains(&i) {
                            // This shouldn't happen for dirs, but just in case
                            filtered_indices.push(filtered_items.len());
                        }
                        filtered_items.push(item.clone());
                    }
                }
            }
            // Prune trailing non-selectable items (empty dirs/headers)
            // Walk backwards and remove any dir/header that has no selectable item after it
            let mut keep = vec![false; filtered_items.len()];
            let selectable_set: std::collections::HashSet<usize> =
                filtered_indices.iter().copied().collect();
            let mut seen_selectable = false;
            for i in (0..filtered_items.len()).rev() {
                if selectable_set.contains(&i) {
                    seen_selectable = true;
                    keep[i] = true;
                } else {
                    // dir/header: keep only if a selectable follows
                    keep[i] = seen_selectable;
                    if !seen_selectable {
                        // Reset for next group
                    }
                    if matches!(filtered_items[i], ListItem::SectionHeader(_)) {
                        seen_selectable = false;
                    }
                }
            }
            // Rebuild with only kept items
            self.display_items.clear();
            self.selectable_indices.clear();
            for (i, item) in filtered_items.into_iter().enumerate() {
                if keep[i] {
                    if selectable_set.contains(&i) {
                        self.selectable_indices.push(self.display_items.len());
                    }
                    self.display_items.push(item);
                }
            }
        }

        if self.selected >= self.selectable_indices.len() {
            self.selected = self.selectable_indices.len().saturating_sub(1);
        }
    }

    pub fn confirm_search(&mut self) {
        self.mode = self.return_to_list_mode();
    }

    pub fn clear_search(&mut self) {
        self.search_input.clear();
        self.apply_search_filter();
        self.mode = self.return_to_list_mode();
    }

    pub fn selected_display_item(&self) -> Option<&ListItem> {
        self.selectable_indices
            .get(self.selected)
            .and_then(|&idx| self.display_items.get(idx))
    }

    fn attach_session(&mut self, name: &str, pid_name: &str, term_rows: u16, term_cols: u16) {
        if let Some(ref current) = self.current_session {
            if *current == pid_name {
                self.set_status("Already in this session".to_string());
                return;
            }
        }
        let (pty_rows, pty_cols) = self.pty_area(term_rows, term_cols);
        let rc = crate::screen::ensure_screenrc();
        self.screen_history_left = crate::screen::dump_scrollback(pid_name);
        match PtySession::spawn("screen", &["-c", &rc, "-d", "-r", pid_name], pty_rows, pty_cols) {
            Ok(pty) => {
                self.pty_session = Some(pty);
                self.attached_name = name.to_string();
                self.record_opened(name);
                self.mode = Mode::Attached;
            }
            Err(e) => {
                self.set_status(format!("Failed to attach: {e}"));
            }
        }
    }

    fn attach_two_pane(
        &mut self,
        name: &str,
        path: &std::path::Path,
        term_rows: u16,
        term_cols: u16,
    ) {
        let right_name = format!("{name}-2");

        let left_pid = self
            .all_sessions
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.pid_name.clone());
        let right_pid = self
            .all_sessions
            .iter()
            .find(|s| s.name == right_name)
            .map(|s| s.pid_name.clone());

        let (pty_rows, total_inner_cols) = self.pty_area(term_rows, term_cols);
        // 60/40 split with 1-col separator
        let left_cols = (total_inner_cols.saturating_sub(1)) * 60 / 100;
        let right_cols = total_inner_cols.saturating_sub(1).saturating_sub(left_cols);

        let rc = crate::screen::ensure_screenrc();

        // Dump scrollback for existing sessions before attaching
        self.screen_history_left = left_pid.as_ref()
            .map(|pid| crate::screen::dump_scrollback(pid))
            .unwrap_or_default();
        self.screen_history_right = right_pid.as_ref()
            .map(|pid| crate::screen::dump_scrollback(pid))
            .unwrap_or_default();

        // Spawn left PTY: create+attach for new sessions, reattach for existing
        let left_result = if let Some(ref pid) = left_pid {
            PtySession::spawn("screen", &["-c", &rc, "-d", "-r", pid], pty_rows, left_cols)
        } else {
            PtySession::spawn_in_dir("screen", &["-c", &rc, "-S", name], pty_rows, left_cols, path)
        };
        match left_result {
            Ok(pty) => {
                self.pty_session = Some(pty);
            }
            Err(e) => {
                self.set_status(format!("Failed to attach left: {e}"));
                return;
            }
        }

        // Spawn right PTY
        let right_result = if let Some(ref pid) = right_pid {
            PtySession::spawn("screen", &["-c", &rc, "-d", "-r", pid], pty_rows, right_cols)
        } else {
            PtySession::spawn_in_dir(
                "screen",
                &["-c", &rc, "-S", &right_name],
                pty_rows,
                right_cols,
                path,
            )
        };
        match right_result {
            Ok(pty) => {
                self.pty_right = Some(pty);
            }
            Err(e) => {
                self.pty_session = None; // clean up left
                self.set_status(format!("Failed to attach right: {e}"));
                return;
            }
        }

        self.attached_name = name.to_string();
        self.attached_right_name = right_name;
        self.active_pane = Pane::Left;
        self.record_opened(name);
        self.mode = Mode::Attached;
    }

    pub fn attach_selected(&mut self, term_rows: u16, term_cols: u16) {
        let item = match self.selected_display_item() {
            Some(item) => item.clone(),
            None => return,
        };
        match item {
            ListItem::SessionItem(session) => {
                let name = session.name.clone();
                let pid_name = session.pid_name.clone();
                self.attach_session(&name, &pid_name, term_rows, term_cols);
            }
            ListItem::TreeRepo {
                name, path, session, ..
            } => {
                if self.workspace_dir.is_some() {
                    // Two-pane mode for workspace repos
                    self.attach_two_pane(&name, &path, term_rows, term_cols);
                } else if let Some(session) = session {
                    self.attach_session(&session.name, &session.pid_name, term_rows, term_cols);
                } else {
                    // Create + attach in one step so the shell starts at the correct PTY size
                    let (pty_rows, pty_cols) = self.pty_area(term_rows, term_cols);
                    let rc = crate::screen::ensure_screenrc();
                    match PtySession::spawn_in_dir(
                        "screen",
                        &["-c", &rc, "-S", &name],
                        pty_rows,
                        pty_cols,
                        &path,
                    ) {
                        Ok(pty) => {
                            self.pty_session = Some(pty);
                            self.record_opened(&name);
                            self.attached_name = name;
                            self.mode = Mode::Attached;
                        }
                        Err(e) => {
                            self.set_status(format!("Error: {e}"));
                        }
                    }
                }
            }
            ListItem::SectionHeader(_) | ListItem::TreeDir { .. } => {}
        }
    }

    pub fn detach_pty(&mut self) {
        // Save snapshot before detaching (while vt100 screen is still accessible)
        if let Some(ref pty) = self.pty_session {
            if !self.attached_name.is_empty() {
                save_snapshot(&self.attached_name, pty.screen());
            }
        }
        if let Some(ref pty) = self.pty_right {
            if !self.attached_right_name.is_empty() {
                save_snapshot(&self.attached_right_name, pty.screen());
            }
        }

        // Send Ctrl+A, d to cleanly detach screen sessions
        if let Some(ref pty) = self.pty_session {
            pty.write_all(b"\x01d");
        }
        if let Some(ref pty) = self.pty_right {
            pty.write_all(b"\x01d");
        }
        // Wait for screen clients to exit cleanly (with timeout)
        let deadline = Instant::now() + Duration::from_millis(50);
        while Instant::now() < deadline {
            let left_done = self
                .pty_session
                .as_mut()
                .map_or(true, |p| !p.is_running());
            let right_done = self.pty_right.as_mut().map_or(true, |p| !p.is_running());
            if left_done && right_done {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }

        self.pty_session = None;
        self.pty_right = None;
        self.screen_history_left.clear();
        self.screen_history_right.clear();
        self.attached_name.clear();
        self.attached_right_name.clear();
        self.active_pane = Pane::Left;
        self.sidebar_focus = SidebarFocus::List;
        self.mode = Mode::Normal;
        self.refresh_sessions();
    }

    pub fn swap_pane(&mut self) {
        if self.pty_right.is_some() {
            self.active_pane = match self.active_pane {
                Pane::Left => Pane::Right,
                Pane::Right => Pane::Left,
            };
        }
    }

    pub fn resize_pty(&mut self, term_rows: u16, term_cols: u16) {
        let (pty_rows, total_inner_cols) = self.pty_area(term_rows, term_cols);

        if self.pty_right.is_some() {
            // Two-pane mode: recalculate 60/40 split
            let left_cols = (total_inner_cols.saturating_sub(1)) * 60 / 100;
            let right_cols = total_inner_cols.saturating_sub(1).saturating_sub(left_cols);

            if let Some(ref mut pty) = self.pty_session {
                pty.resize(pty_rows, left_cols);
            }
            if let Some(ref mut pty) = self.pty_right {
                pty.resize(pty_rows, right_cols);
            }
        } else {
            // Single pane
            if let Some(ref mut pty) = self.pty_session {
                pty.resize(pty_rows, total_inner_cols);
            }
        }
    }

    /// Return to the appropriate mode: Attached if PTY is alive (sidebar), Normal otherwise.
    fn return_to_list_mode(&self) -> Mode {
        if self.pty_session.is_some() {
            Mode::Attached
        } else {
            Mode::Normal
        }
    }

    pub fn start_create(&mut self) {
        self.mode = Mode::Creating;
        self.create_input.clear();
        self.cursor_pos = 0;
    }

    pub fn confirm_create(&mut self) {
        let name = self.create_input.trim().to_string();
        if name.is_empty() {
            self.mode = self.return_to_list_mode();
            return;
        }
        match screen::create_session(&name) {
            Ok(()) => {
                self.set_status(format!("Created session '{name}'"));
                self.refresh_sessions();
            }
            Err(e) => {
                self.set_status(format!("Error: {e}"));
            }
        }
        self.mode = self.return_to_list_mode();
    }

    pub fn cancel_create(&mut self) {
        self.mode = self.return_to_list_mode();
    }

    pub fn start_rename(&mut self) {
        if let Some(ListItem::SessionItem(session)) = self.selected_display_item().cloned() {
            self.rename_pid_name = session.pid_name;
            self.create_input = session.name;
            self.cursor_pos = self.create_input.chars().count();
            self.mode = Mode::Renaming;
        }
    }

    pub fn confirm_rename(&mut self) {
        let new_name = self.create_input.trim().to_string();
        if new_name.is_empty() {
            self.mode = self.return_to_list_mode();
            return;
        }
        match screen::rename_session(&self.rename_pid_name, &new_name) {
            Ok(()) => {
                self.set_status(format!("Renamed to '{new_name}'"));
                self.refresh_sessions();
            }
            Err(e) => {
                self.set_status(format!("Error: {e}"));
            }
        }
        self.mode = self.return_to_list_mode();
    }

    pub fn cancel_rename(&mut self) {
        self.mode = self.return_to_list_mode();
    }

    pub fn start_kill(&mut self) {
        let info = match self.selected_display_item() {
            Some(ListItem::SessionItem(session)) => {
                Some((session.name.clone(), session.pid_name.clone()))
            }
            Some(ListItem::TreeRepo { session: Some(session), .. }) => {
                Some((session.name.clone(), session.pid_name.clone()))
            }
            _ => None,
        };
        if let Some(info) = info {
            self.kill_session_info = Some(info);
            self.mode = Mode::ConfirmKill;
        }
    }

    pub fn confirm_kill(&mut self) {
        if let Some((name, pid_name)) = self.kill_session_info.take() {
            match screen::kill_session(&pid_name) {
                Ok(()) => {
                    self.set_status(format!("Killed '{name}'"));
                    self.refresh_sessions();
                }
                Err(e) => {
                    self.set_status(format!("Error: {e}"));
                }
            }
        }
        self.mode = self.return_to_list_mode();
    }

    pub fn cancel_kill(&mut self) {
        self.kill_session_info = None;
        self.mode = self.return_to_list_mode();
    }

    pub fn start_kill_all(&mut self) {
        if !self.all_sessions.is_empty() {
            self.mode = Mode::ConfirmKillAll1;
        }
    }

    pub fn confirm_kill_all_step1(&mut self) {
        self.mode = Mode::ConfirmKillAll2;
    }

    pub fn confirm_kill_all_step2(&mut self) {
        let mut killed = 0;
        let mut errors = Vec::new();
        for session in self.all_sessions.clone() {
            if self.is_current_session(&session) {
                continue;
            }
            match screen::kill_session(&session.pid_name) {
                Ok(()) => killed += 1,
                Err(e) => errors.push(e),
            }
        }
        if errors.is_empty() {
            self.set_status(format!("Killed {killed} sessions"));
        } else {
            self.set_status(format!("Killed {killed}, {} errors", errors.len()));
        }
        self.refresh_sessions();
        self.mode = self.return_to_list_mode();
    }

    pub fn cancel_kill_all(&mut self) {
        self.mode = self.return_to_list_mode();
    }

    pub fn go_home(&mut self) {
        if self.current_session.is_some() {
            self.action = Action::GoHome;
        }
    }

    pub fn is_current_session(&self, session: &Session) -> bool {
        self.current_session
            .as_ref()
            .is_some_and(|current| *current == session.pid_name)
    }

    /// Sidebar width: 20% of terminal, clamped to [20, 40].
    pub fn sidebar_width(&self, term_cols: u16) -> u16 {
        let w = term_cols / 5;
        w.clamp(20, 40)
    }

    /// Effective PTY area when sidebar is active, subtracting sidebar + its border.
    /// Returns (effective_rows, effective_cols).
    pub fn pty_area(&self, term_rows: u16, term_cols: u16) -> (u16, u16) {
        if self.sidebar_mode {
            let sw = self.sidebar_width(term_cols);
            // sidebar takes sw cols (including its right border), content area gets the rest
            // content area has its own 1-col border on right, 1 row top + 1 row bottom
            let content_cols = term_cols.saturating_sub(sw);
            let pty_cols = content_cols.saturating_sub(2); // left border + right border of content box
            let pty_rows = term_rows.saturating_sub(2);
            (pty_rows, pty_cols)
        } else {
            (term_rows.saturating_sub(2), term_cols.saturating_sub(2))
        }
    }

    /// Switch to a different session in sidebar mode: detach current, attach selected.
    pub fn sidebar_switch_session(&mut self, term_rows: u16, term_cols: u16) {
        // Detach current PTY without leaving Attached mode
        if let Some(ref pty) = self.pty_session {
            if !self.attached_name.is_empty() {
                save_snapshot(&self.attached_name, pty.screen());
            }
            pty.write_all(b"\x01d");
        }
        if let Some(ref pty) = self.pty_right {
            if !self.attached_right_name.is_empty() {
                save_snapshot(&self.attached_right_name, pty.screen());
            }
            pty.write_all(b"\x01d");
        }
        // Wait briefly for clean detach
        let deadline = Instant::now() + Duration::from_millis(50);
        while Instant::now() < deadline {
            let left_done = self.pty_session.as_mut().map_or(true, |p| !p.is_running());
            let right_done = self.pty_right.as_mut().map_or(true, |p| !p.is_running());
            if left_done && right_done { break; }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.pty_session = None;
        self.pty_right = None;
        self.screen_history_left.clear();
        self.screen_history_right.clear();
        self.attached_name.clear();
        self.attached_right_name.clear();
        self.active_pane = Pane::Left;
        self.mode = Mode::Normal;
        self.refresh_sessions();

        // Now attach the selected session
        self.attach_selected(term_rows, term_cols);
        if self.mode == Mode::Attached {
            self.sidebar_focus = SidebarFocus::Content;
        }
    }

    fn record_opened(&mut self, name: &str) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.history.insert(name.to_string(), ts);
        save_history(&self.history);
    }

    /// Format the last-opened time for display, returning None if never opened.
    pub fn last_opened(&self, name: &str) -> Option<String> {
        let ts = *self.history.get(name)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Some(format_relative(now, ts))
    }

}

fn snapshot_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("scrn")
        .join("snapshots")
}

fn save_snapshot(name: &str, screen: &vt100::Screen) {
    let dir = snapshot_dir();
    let _ = std::fs::create_dir_all(&dir);

    let (rows, cols) = screen.size();
    let mut text = String::new();
    for row in 0..rows {
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let c = cell.contents();
                if c.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&c);
                }
            }
        }
        text.push_str(line.trim_end());
        text.push('\n');
    }
    let _ = std::fs::write(dir.join(name), text);
}

fn build_tree_prefix(guide_lines: &[bool], is_last: bool) -> String {
    let mut prefix = String::new();
    for &has_more in guide_lines {
        if has_more {
            prefix.push_str("\u{2502}  "); // │  (3 chars)
        } else {
            prefix.push_str("   "); // 3 spaces
        }
    }
    if is_last {
        prefix.push_str("\u{2514}\u{2500} "); // └─ (3 chars)
    } else {
        prefix.push_str("\u{251c}\u{2500} "); // ├─ (3 chars)
    }
    prefix
}

fn flatten_tree(
    node: &TreeNode,
    depth: usize,
    session_map: &std::collections::HashMap<&str, &Session>,
    merged: &mut std::collections::HashSet<String>,
    display_items: &mut Vec<ListItem>,
    selectable_indices: &mut Vec<usize>,
    guide_lines: &mut Vec<bool>,
) {
    if !node.is_repo && depth == 0 {
        display_items.push(ListItem::TreeDir {
            name: node.name.clone(),
            prefix: String::new(),
        });
    }

    let child_count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let is_last = i == child_count - 1;
        let prefix = build_tree_prefix(guide_lines, is_last);

        if child.is_repo {
            let session = session_map.get(child.name.as_str()).cloned().cloned();
            if let Some(ref s) = session {
                merged.insert(s.name.clone());
            }
            let companion = format!("{}-2", child.name);
            if session_map.contains_key(companion.as_str()) {
                merged.insert(companion);
            }

            let idx = display_items.len();
            display_items.push(ListItem::TreeRepo {
                name: child.name.clone(),
                path: child.path.clone(),
                session,
                prefix,
            });
            selectable_indices.push(idx);
        } else {
            display_items.push(ListItem::TreeDir {
                name: child.name.clone(),
                prefix,
            });
            guide_lines.push(!is_last);
            flatten_tree(child, depth + 1, session_map, merged, display_items, selectable_indices, guide_lines);
            guide_lines.pop();
        }
    }
}

fn flatten_filtered(
    node: &TreeNode,
    depth: usize,
    query: &str,
    session_map: &std::collections::HashMap<&str, &Session>,
    merged: &mut std::collections::HashSet<String>,
    display_items: &mut Vec<ListItem>,
    selectable_indices: &mut Vec<usize>,
    guide_lines: &mut Vec<bool>,
) {
    if !node.is_repo {
        if !tree_has_match(node, query) {
            return;
        }
        if depth == 0 {
            display_items.push(ListItem::TreeDir {
                name: node.name.clone(),
                prefix: String::new(),
            });
        }
    }

    // Pre-filter visible children
    let visible_children: Vec<&TreeNode> = node
        .children
        .iter()
        .filter(|child| {
            if child.is_repo {
                fuzzy_match(&child.name, query).is_some()
            } else {
                tree_has_match(child, query)
            }
        })
        .collect();

    let visible_count = visible_children.len();
    for (i, child) in visible_children.iter().enumerate() {
        let is_last = i == visible_count - 1;
        let prefix = build_tree_prefix(guide_lines, is_last);

        if child.is_repo {
            let session = session_map.get(child.name.as_str()).cloned().cloned();
            if let Some(ref s) = session {
                merged.insert(s.name.clone());
            }
            let companion = format!("{}-2", child.name);
            if session_map.contains_key(companion.as_str()) {
                merged.insert(companion);
            }

            let idx = display_items.len();
            display_items.push(ListItem::TreeRepo {
                name: child.name.clone(),
                path: child.path.clone(),
                session,
                prefix,
            });
            selectable_indices.push(idx);
        } else {
            display_items.push(ListItem::TreeDir {
                name: child.name.clone(),
                prefix,
            });
            guide_lines.push(!is_last);
            flatten_filtered(child, depth + 1, query, session_map, merged, display_items, selectable_indices, guide_lines);
            guide_lines.pop();
        }
    }
}

/// Check if any repo descendant of this node matches the query.
fn tree_has_match(node: &TreeNode, query: &str) -> bool {
    for child in &node.children {
        if child.is_repo {
            if fuzzy_match(&child.name, query).is_some() {
                return true;
            }
        } else if tree_has_match(child, query) {
            return true;
        }
    }
    false
}

fn pins_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("scrn")
        .join("pins")
}

fn load_pins() -> HashSet<String> {
    let path = pins_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    contents
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

fn save_pins(pins: &HashSet<String>) {
    let path = pins_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<&str> = pins.iter().map(|s| s.as_str()).collect();
    lines.sort();
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

fn history_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("scrn")
        .join("history")
}

fn sessions_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("scrn")
        .join("sessions")
}

fn load_history() -> HashMap<String, u64> {
    let path = history_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let mut map = HashMap::new();
    for line in contents.lines() {
        if let Some((name, ts_str)) = line.split_once('\t') {
            if let Ok(ts) = ts_str.parse::<u64>() {
                map.insert(name.to_string(), ts);
            }
        }
    }
    map
}

fn save_history(history: &HashMap<String, u64>) {
    let path = history_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<String> = history
        .iter()
        .map(|(name, ts)| format!("{name}\t{ts}"))
        .collect();
    lines.sort();
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

fn collect_repo_paths(node: &TreeNode, map: &mut HashMap<String, PathBuf>) {
    if node.is_repo {
        map.insert(node.name.clone(), node.path.clone());
    }
    for child in &node.children {
        collect_repo_paths(child, map);
    }
}

fn save_sessions(all_sessions: &[Session], workspace_tree: &Option<TreeNode>) {
    let path = sessions_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());

    let mut repo_paths: HashMap<String, PathBuf> = HashMap::new();
    if let Some(ref tree) = workspace_tree {
        collect_repo_paths(tree, &mut repo_paths);
    }

    let mut lines: Vec<String> = all_sessions
        .iter()
        .filter(|s| !s.name.ends_with("-2"))
        .filter(|s| !s.name.starts_with("tty") && !s.name.starts_with("pts"))
        .map(|s| {
            if let Some(p) = repo_paths.get(&s.name) {
                format!("{}\t{}", s.name, p.display())
            } else {
                s.name.clone()
            }
        })
        .collect();
    lines.sort();
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

fn load_saved_sessions() -> Vec<(String, Option<PathBuf>)> {
    let path = sessions_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    contents
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            if let Some((name, path_str)) = line.split_once('\t') {
                (name.to_string(), Some(PathBuf::from(path_str)))
            } else {
                (line.to_string(), None)
            }
        })
        .collect()
}

fn format_relative(now: u64, ts: u64) -> String {
    let delta = now.saturating_sub(ts);
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        let m = delta / 60;
        if m == 1 { "1 min ago".to_string() } else { format!("{m} mins ago") }
    } else if delta < 86400 {
        let h = delta / 3600;
        if h == 1 { "1 hour ago".to_string() } else { format!("{h} hours ago") }
    } else if delta < 86400 * 30 {
        let d = delta / 86400;
        if d == 1 { "1 day ago".to_string() } else { format!("{d} days ago") }
    } else {
        let d = delta / 86400;
        let m = d / 30;
        if m == 1 { "1 month ago".to_string() } else { format!("{m} months ago") }
    }
}

pub fn fuzzy_match(haystack: &str, needle: &str) -> Option<(Vec<usize>, i32)> {
    let haystack_lower: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    let needle_lower: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();

    if needle_lower.is_empty() {
        return Some((Vec::new(), 0));
    }

    // Try exact substring match first
    if let Some(start) = find_substring_pos(&haystack_lower, &needle_lower) {
        let positions: Vec<usize> = (start..start + needle_lower.len()).collect();
        let mut score: i32 = 10000;
        if start == 0 {
            score += 5000; // prefix bonus
        }
        if start == 0
            || haystack_lower[start - 1] == '-'
            || haystack_lower[start - 1] == '_'
            || haystack_lower[start - 1] == '/'
            || haystack_lower[start - 1] == '.'
            || haystack_lower[start - 1] == ' '
        {
            score += 3000; // word boundary bonus
        }
        return Some((positions, score));
    }

    // Fall back to greedy subsequence match
    let mut positions = Vec::with_capacity(needle_lower.len());
    let mut hay_idx = 0;
    for nc in &needle_lower {
        let mut found = false;
        while hay_idx < haystack_lower.len() {
            if haystack_lower[hay_idx] == *nc {
                positions.push(hay_idx);
                hay_idx += 1;
                found = true;
                break;
            }
            hay_idx += 1;
        }
        if !found {
            return None;
        }
    }

    // Score: consecutive bonus + word boundary bonus - spread penalty
    let mut score: i32 = 0;
    for i in 0..positions.len() {
        if i > 0 && positions[i] == positions[i - 1] + 1 {
            score += 50; // consecutive
        }
        let pos = positions[i];
        if pos == 0
            || haystack_lower[pos - 1] == '-'
            || haystack_lower[pos - 1] == '_'
            || haystack_lower[pos - 1] == '/'
            || haystack_lower[pos - 1] == '.'
            || haystack_lower[pos - 1] == ' '
        {
            score += 20; // word boundary
        }
    }
    let spread = positions.last().unwrap_or(&0) - positions.first().unwrap_or(&0);
    score -= spread as i32;

    Some((positions, score))
}

fn find_substring_pos(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    'outer: for start in 0..=haystack.len() - needle.len() {
        for (i, nc) in needle.iter().enumerate() {
            if haystack[start + i] != *nc {
                continue 'outer;
            }
        }
        return Some(start);
    }
    None
}
