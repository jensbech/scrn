use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::screen::{self, Session};
use crate::workspace::{self, TreeNode};

#[derive(PartialEq)]
pub enum Mode {
    Normal,
    Searching,
    Creating,
    Renaming,
    ConfirmPin,
    ConfirmConstant,
    ConfirmKill,
    ConfirmKillAll1,
    ConfirmKillAll2,
    ConfirmQuit,
}

pub enum Action {
    None,
    Attach(String),                   // pid.name
    Create(String, Option<PathBuf>),  // name, optional dir
    Quit,
}

#[derive(Clone)]
pub enum ListItem {
    SectionHeader(String),
    Separator,
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
    pub action: Action,
    pub rename_pid_name: String,
    pub workspace_dir: Option<PathBuf>,
    pub workspace_tree: Option<TreeNode>,
    pub display_items: Vec<ListItem>,
    pub selectable_indices: Vec<usize>,
    pub pin_target: Option<String>,
    pub constant_target: Option<String>,
    pub kill_session_info: Option<(String, String)>,
    pub pre_search_selected: usize,
    pub search_filter_active: bool,
    /// screen PID -> foreground process name (empty = idle shell)
    pub foreground_procs: HashMap<u32, String>,
    /// session name -> unix timestamp of last attach
    pub history: HashMap<String, u64>,
    pub filter_opened: bool,
    /// pinned session/repo names — always shown at top
    pub pins: HashSet<String>,
    /// constant session/repo names — always shown above pinned
    pub constants: HashSet<String>,
    pub table_data_y: u16,
    pub table_data_end_y: u16,
    pub table_scroll_offset: usize,
    pub last_click: Option<(Instant, usize)>,
}

impl App {
    pub fn new(workspace: Option<PathBuf>) -> Self {
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
            action: Action::None,
            rename_pid_name: String::new(),
            workspace_dir: workspace,
            workspace_tree: None,
            display_items: Vec::new(),
            selectable_indices: Vec::new(),
            pin_target: None,
            constant_target: None,
            kill_session_info: None,
            pre_search_selected: 0,
            search_filter_active: true,
            foreground_procs: HashMap::new(),
            history: load_history(),
            filter_opened: false,
            pins: load_pins(),
            constants: load_constants(),
            table_data_y: 0,
            table_data_end_y: 0,
            table_scroll_offset: 0,
            last_click: None,
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
        let pids: Vec<u32> = self.all_sessions
            .iter()
            .filter_map(|s| s.pid_name.split('.').next()?.parse().ok())
            .collect();
        self.foreground_procs = screen::get_foreground_processes(&pids);
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

    pub fn start_pin_confirm(&mut self) {
        let name = match self.selected_display_item() {
            Some(ListItem::TreeRepo { name, .. }) => name.clone(),
            Some(ListItem::SessionItem(session)) => session.name.clone(),
            _ => return,
        };
        self.pin_target = Some(name);
        self.mode = Mode::ConfirmPin;
    }

    pub fn confirm_pin(&mut self) {
        if let Some(name) = self.pin_target.take() {
            if self.pins.contains(&name) {
                self.pins.remove(&name);
                self.set_status(format!("Unpinned '{name}'"));
            } else {
                self.pins.insert(name.clone());
                // Mutual exclusivity: remove from constants if present
                if self.constants.remove(&name) {
                    save_constants(&self.constants);
                }
                self.set_status(format!("Pinned '{name}'"));
            }
            save_pins(&self.pins);
            self.rebuild_display_list();
        }
        self.mode = Mode::Normal;
    }

    pub fn cancel_pin(&mut self) {
        self.pin_target = None;
        self.mode = Mode::Normal;
    }

    pub fn start_constant_confirm(&mut self) {
        let name = match self.selected_display_item() {
            Some(ListItem::TreeRepo { name, .. }) => name.clone(),
            Some(ListItem::SessionItem(session)) => session.name.clone(),
            _ => return,
        };
        self.constant_target = Some(name);
        self.mode = Mode::ConfirmConstant;
    }

    pub fn confirm_constant(&mut self) {
        if let Some(name) = self.constant_target.take() {
            if self.constants.contains(&name) {
                self.constants.remove(&name);
                self.set_status(format!("Removed from constants '{name}'"));
            } else {
                self.constants.insert(name.clone());
                if self.pins.remove(&name) {
                    save_pins(&self.pins);
                }
                self.set_status(format!("Added to constants '{name}'"));
            }
            save_constants(&self.constants);
            self.rebuild_display_list();
        }
        self.mode = Mode::Normal;
    }

    pub fn cancel_constant(&mut self) {
        self.constant_target = None;
        self.mode = Mode::Normal;
    }

    pub fn start_search(&mut self) {
        self.mode = Mode::Searching;
        self.search_input.clear();
        self.pre_search_selected = self.selected;
        self.search_filter_active = true;
    }

    pub fn apply_search_filter(&mut self) {
        let filter_active = self.search_filter_active && !self.search_input.is_empty();
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
                if !filter_active {
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

    pub fn toggle_search_filter(&mut self) {
        self.search_filter_active = !self.search_filter_active;
        self.apply_search_filter();
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

        let filter_active = self.search_filter_active && !self.search_input.is_empty();

        if let Some(ref tree) = self.workspace_tree {
            let tree = tree.clone();
            if !filter_active {
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
        let mut all_orphan_sessions: Vec<&Session> = sessions_clone
            .iter()
            .filter(|s| {
                !merged_sessions.contains(&s.name)
                    && !s.name.ends_with("-2")
            })
            .collect();

        // Sort by fuzzy match score when searching
        if filter_active {
            all_orphan_sessions.sort_by(|a, b| {
                let score_a = fuzzy_match(&a.name, &self.search_input)
                    .map(|(_, s)| s)
                    .unwrap_or(i32::MIN);
                let score_b = fuzzy_match(&b.name, &self.search_input)
                    .map(|(_, s)| s)
                    .unwrap_or(i32::MIN);
                score_b.cmp(&score_a)
            });
        }

        // Split throwaway sessions (tmp-*) from regular orphans
        let orphan_sessions: Vec<&Session> = all_orphan_sessions.iter()
            .filter(|s| !s.name.starts_with("tmp-"))
            .copied()
            .collect();
        let throwaway_sessions: Vec<&Session> = all_orphan_sessions.iter()
            .filter(|s| s.name.starts_with("tmp-"))
            .copied()
            .collect();

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

        // Build throwaway items (always appended last)
        let mut throwaway_items: Vec<ListItem> = Vec::new();
        let mut throwaway_selectable: Vec<usize> = Vec::new();

        if !throwaway_sessions.is_empty() {
            throwaway_items.push(ListItem::SectionHeader("Throwaway".to_string()));
            for session in &throwaway_sessions {
                let idx = throwaway_items.len();
                throwaway_items.push(ListItem::SessionItem((*session).clone()));
                throwaway_selectable.push(idx);
            }
        }

        // Helper: extract items matching a name set from ws_items and orphan_items
        // Returns (extracted_items, extracted_selectable, ws_indices_to_remove, orphan_indices_to_remove)
        fn extract_section(
            name_set: &HashSet<String>,
            ws_items: &[ListItem],
            orphan_items: &[ListItem],
        ) -> (Vec<ListItem>, Vec<usize>, HashSet<usize>, HashSet<usize>) {
            let mut items: Vec<ListItem> = Vec::new();
            let mut selectable: Vec<usize> = Vec::new();
            let mut ws_remove: HashSet<usize> = HashSet::new();
            let mut orphan_remove: HashSet<usize> = HashSet::new();

            // Find matching ws repos and track their TreeDir parents
            let mut needed_dir_indices: HashSet<usize> = HashSet::new();
            let mut last_dir_idx: Option<usize> = None;
            for (i, item) in ws_items.iter().enumerate() {
                match item {
                    ListItem::TreeDir { .. } | ListItem::SectionHeader(_) => {
                        last_dir_idx = Some(i);
                    }
                    ListItem::TreeRepo { name, .. } => {
                        if name_set.contains(name.as_str()) {
                            ws_remove.insert(i);
                            if let Some(dir_idx) = last_dir_idx {
                                needed_dir_indices.insert(dir_idx);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Build items preserving tree order (dirs + repos)
            for (i, item) in ws_items.iter().enumerate() {
                if needed_dir_indices.contains(&i) {
                    items.push(item.clone());
                } else if ws_remove.contains(&i) {
                    selectable.push(items.len());
                    items.push(item.clone());
                }
            }

            // Extract matching SessionItems from orphan_items
            for (i, item) in orphan_items.iter().enumerate() {
                if let ListItem::SessionItem(session) = item {
                    if name_set.contains(session.name.as_str()) {
                        orphan_remove.insert(i);
                        selectable.push(items.len());
                        items.push(item.clone());
                    }
                }
            }

            (items, selectable, ws_remove, orphan_remove)
        }

        // Helper: remove indices from a group
        fn remove_indices(
            items: &mut Vec<ListItem>,
            selectable: &mut Vec<usize>,
            remove_set: &HashSet<usize>,
            old_items: Vec<ListItem>,
            old_selectable: Vec<usize>,
        ) {
            let sel_set: HashSet<usize> = old_selectable.into_iter().collect();
            for (i, item) in old_items.into_iter().enumerate() {
                if remove_set.contains(&i) {
                    continue;
                }
                let new_idx = items.len();
                if sel_set.contains(&i) {
                    selectable.push(new_idx);
                }
                items.push(item);
            }
        }

        // Extract constants section (highest priority, claimed first)
        let constants = &self.constants;
        let (const_items, const_selectable, const_ws_remove, const_orphan_remove) =
            extract_section(constants, &ws_items, &orphan_items);

        // Remove constant items from ws/orphan groups
        if !const_ws_remove.is_empty() {
            let old_ws = std::mem::take(&mut ws_items);
            let old_sel = std::mem::take(&mut ws_selectable);
            remove_indices(&mut ws_items, &mut ws_selectable, &const_ws_remove, old_ws, old_sel);
        }
        if !const_orphan_remove.is_empty() {
            let old_orphan = std::mem::take(&mut orphan_items);
            let old_sel = std::mem::take(&mut orphan_selectable);
            remove_indices(&mut orphan_items, &mut orphan_selectable, &const_orphan_remove, old_orphan, old_sel);
        }

        // Extract pinned section (excludes items already claimed by constants)
        let pins = &self.pins;
        let (pinned_items, pinned_selectable, pinned_ws_remove, pinned_orphan_remove) =
            extract_section(pins, &ws_items, &orphan_items);

        // Remove pinned items from ws/orphan groups
        if !pinned_ws_remove.is_empty() {
            let old_ws = std::mem::take(&mut ws_items);
            let old_sel = std::mem::take(&mut ws_selectable);
            remove_indices(&mut ws_items, &mut ws_selectable, &pinned_ws_remove, old_ws, old_sel);
        }
        if !pinned_orphan_remove.is_empty() {
            let old_orphan = std::mem::take(&mut orphan_items);
            let old_sel = std::mem::take(&mut orphan_selectable);
            remove_indices(&mut orphan_items, &mut orphan_selectable, &pinned_orphan_remove, old_orphan, old_sel);
        }

        // Prune orphan section if all items were extracted (only header remains)
        if orphan_selectable.is_empty() {
            orphan_items.clear();
        }

        // Append constants section first (no header — the gold coloring is enough)
        if !const_items.is_empty() {
            let offset = self.display_items.len();
            for idx in const_selectable {
                self.selectable_indices.push(offset + idx);
            }
            self.display_items.extend(const_items);
            self.display_items.push(ListItem::Separator);
        }

        // Append pinned section
        if !pinned_items.is_empty() {
            self.display_items.push(ListItem::SectionHeader("Pinned".to_string()));
            let offset = self.display_items.len();
            for idx in pinned_selectable {
                self.selectable_indices.push(offset + idx);
            }
            self.display_items.extend(pinned_items);
            self.display_items.push(ListItem::Separator);
        }

        // When searching, hoist the group with the best match score first
        let orphans_first = if filter_active {
            let best_ws_score = ws_items.iter().filter_map(|item| {
                if let ListItem::TreeRepo { name, .. } = item {
                    fuzzy_match(name, &self.search_input).map(|(_, s)| s)
                } else {
                    None
                }
            }).max().unwrap_or(i32::MIN);

            let best_orphan_score = all_orphan_sessions.iter().filter_map(|s| {
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

        // Throwaway sessions always go at the very bottom
        append_group(throwaway_items, throwaway_selectable, &mut self.display_items, &mut self.selectable_indices);

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
                    ListItem::TreeDir { .. } | ListItem::SectionHeader(_) | ListItem::Separator => {
                        if self.selectable_indices.contains(&i) {
                            filtered_indices.push(filtered_items.len());
                        }
                        filtered_items.push(item.clone());
                    }
                }
            }
            // Prune trailing non-selectable items (empty dirs/headers)
            let mut keep = vec![false; filtered_items.len()];
            let selectable_set: std::collections::HashSet<usize> =
                filtered_indices.iter().copied().collect();
            let mut seen_selectable = false;
            for i in (0..filtered_items.len()).rev() {
                if selectable_set.contains(&i) {
                    seen_selectable = true;
                    keep[i] = true;
                } else {
                    keep[i] = seen_selectable;
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
        self.mode = Mode::Normal;
    }

    pub fn clear_search(&mut self) {
        self.search_input.clear();
        self.search_filter_active = true;
        self.apply_search_filter();
        self.selected = self.pre_search_selected
            .min(self.selectable_indices.len().saturating_sub(1));
        self.mode = Mode::Normal;
    }

    pub fn selected_display_item(&self) -> Option<&ListItem> {
        self.selectable_indices
            .get(self.selected)
            .and_then(|&idx| self.display_items.get(idx))
    }

    /// Set the action to attach or create based on the currently selected item.
    pub fn select_for_attach(&mut self) {
        let item = match self.selected_display_item() {
            Some(item) => item.clone(),
            None => return,
        };
        match item {
            ListItem::SessionItem(session) => {
                self.record_opened(&session.name);
                self.action = Action::Attach(session.pid_name);
            }
            ListItem::TreeRepo { name, path, session, .. } => {
                if let Some(session) = session {
                    self.record_opened(&session.name);
                    self.action = Action::Attach(session.pid_name);
                } else {
                    self.record_opened(&name);
                    self.action = Action::Create(name, Some(path));
                }
            }
            ListItem::SectionHeader(_) | ListItem::TreeDir { .. } | ListItem::Separator => {}
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
            self.mode = Mode::Normal;
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
        self.mode = Mode::Normal;
    }

    pub fn cancel_create(&mut self) {
        self.mode = Mode::Normal;
    }

    pub fn create_throwaway(&mut self) {
        let name = generate_throwaway_name(&self.all_sessions);
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        self.record_opened(&name);
        self.action = Action::Create(name, Some(PathBuf::from(home)));
    }

    pub fn kill_all_throwaway(&mut self) {
        let throwaway: Vec<String> = self.all_sessions
            .iter()
            .filter(|s| s.name.starts_with("tmp-"))
            .map(|s| s.pid_name.clone())
            .collect();
        for pid_name in throwaway {
            let _ = screen::kill_session(&pid_name);
        }
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
            self.mode = Mode::Normal;
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
        self.mode = Mode::Normal;
    }

    pub fn cancel_rename(&mut self) {
        self.mode = Mode::Normal;
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
        self.mode = Mode::Normal;
    }

    pub fn cancel_kill(&mut self) {
        self.kill_session_info = None;
        self.mode = Mode::Normal;
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
        self.mode = Mode::Normal;
    }

    pub fn cancel_kill_all(&mut self) {
        self.mode = Mode::Normal;
    }

    pub fn is_current_session(&self, session: &Session) -> bool {
        self.current_session
            .as_ref()
            .is_some_and(|current| *current == session.pid_name)
    }

    fn record_opened(&mut self, name: &str) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.history.insert(name.to_string(), ts);
        save_history(&self.history);
    }

    /// Return the foreground process name for the session identified by `pid_name`,
    /// or an empty string if the session is at an idle shell prompt.
    pub fn session_proc(&self, pid_name: &str) -> &str {
        let pid: u32 = match pid_name.split('.').next().and_then(|s| s.parse().ok()) {
            Some(p) => p,
            None => return "",
        };
        self.foreground_procs.get(&pid).map(|s| s.as_str()).unwrap_or("")
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

/// Walk down a chain of single-child directory nodes, joining names with `/`.
/// Returns the collapsed display name and the deepest node whose children should be rendered.
fn compact_dir_chain<'a>(node: &'a TreeNode) -> (String, &'a TreeNode) {
    let mut name = node.name.clone();
    let mut current = node;
    loop {
        if current.children.len() == 1 && !current.children[0].is_repo {
            current = &current.children[0];
            name.push('/');
            name.push_str(&current.name);
        } else {
            break;
        }
    }
    (name, current)
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
    let (iter_node, dir_prefix): (&TreeNode, String) = if !node.is_repo && depth == 0 {
        let has_direct_repos = node.children.iter().any(|c| c.is_repo);
        if has_direct_repos {
            let (compact_name, leaf) = compact_dir_chain(node);
            display_items.push(ListItem::TreeDir { name: compact_name, prefix: String::new() });
            (leaf, String::new())
        } else {
            (node, format!("{}/", node.name))
        }
    } else {
        (node, String::new())
    };

    for child in iter_node.children.iter() {
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
                prefix: " ".to_string(),
            });
            selectable_indices.push(idx);
        } else {
            let (compact_name, leaf) = compact_dir_chain(child);
            let full_name = if dir_prefix.is_empty() {
                compact_name
            } else {
                format!("{}{}", dir_prefix, compact_name)
            };
            display_items.push(ListItem::TreeDir { name: full_name, prefix: String::new() });
            guide_lines.push(false);
            flatten_tree(leaf, depth + 1, session_map, merged, display_items, selectable_indices, guide_lines);
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
    }

    let (source_node, dir_prefix): (&TreeNode, String) = if !node.is_repo && depth == 0 {
        let has_direct_repos = node.children.iter().any(|c| c.is_repo && fuzzy_match(&c.name, query).is_some());
        if has_direct_repos {
            let (compact_name, leaf) = compact_dir_chain(node);
            display_items.push(ListItem::TreeDir { name: compact_name, prefix: String::new() });
            (leaf, String::new())
        } else {
            (node, format!("{}/", node.name))
        }
    } else {
        (node, String::new())
    };

    let visible_children: Vec<&TreeNode> = source_node
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

    for child in visible_children.iter() {
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
                prefix: " ".to_string(),
            });
            selectable_indices.push(idx);
        } else {
            let (compact_name, leaf) = compact_dir_chain(child);
            let full_name = if dir_prefix.is_empty() {
                compact_name
            } else {
                format!("{}{}", dir_prefix, compact_name)
            };
            display_items.push(ListItem::TreeDir { name: full_name, prefix: String::new() });
            guide_lines.push(false);
            flatten_filtered(leaf, depth + 1, query, session_map, merged, display_items, selectable_indices, guide_lines);
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

fn constants_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("scrn")
        .join("constants")
}

fn load_constants() -> HashSet<String> {
    let path = constants_path();
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

fn save_constants(constants: &HashSet<String>) {
    let path = constants_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<&str> = constants.iter().map(|s| s.as_str()).collect();
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
        .filter(|s| !s.name.starts_with("tmp-"))
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

fn generate_throwaway_name(all_sessions: &[Session]) -> String {
    const ADJS: &[&str] = &[
        "swift", "calm", "bold", "dark", "pale", "warm", "cold", "wild", "soft", "keen",
        "neat", "raw", "odd", "dry", "old", "vast", "deep", "rich", "slim", "deft",
    ];
    const NOUNS: &[&str] = &[
        "fox", "hawk", "wolf", "bear", "deer", "owl", "crow", "hare", "swan", "wren",
        "kite", "elk", "jay", "cod", "oak", "ash", "elm", "bay", "crag", "glen",
    ];
    let existing: HashSet<&str> = all_sessions.iter().map(|s| s.name.as_str()).collect();
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    for i in 0..200_usize {
        let adj = ADJS[seed.wrapping_add(i.wrapping_mul(17)) % ADJS.len()];
        let noun = NOUNS[seed.wrapping_add(i.wrapping_mul(31)).wrapping_mul(3) % NOUNS.len()];
        let name = format!("tmp-{adj}-{noun}");
        if !existing.contains(name.as_str()) {
            return name;
        }
    }
    format!("tmp-{}", seed % 10000)
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
