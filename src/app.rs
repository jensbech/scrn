use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::git;
use crate::screen::{self, Session};
use crate::workspace::{self, TreeNode};

#[derive(Clone, Debug)]
pub struct WorktreeInfo {
    pub repo_path: PathBuf,
    pub worktree_path: PathBuf,
}

#[derive(PartialEq)]
pub enum Mode {
    Normal,
    Searching,
    Creating,
    ConfirmPin,
    ConfirmConstant,
    ConfirmKill,
    ConfirmKillAll1,
    ConfirmKillAll2,
    ConfirmQuit,
    Ordering,
    ConstantOrdering,
    EditingCommand,
    EditingLabel,
    LabelNewCompanion,
}

pub enum Action {
    None,
    Attach(String),                   // pid.name
    Create(String, Option<PathBuf>),  // name, optional dir
    Quit,
}

/// Data collected by a refresh — can be built on a background thread.
pub struct RefreshData {
    pub sessions: Vec<Session>,
    pub workspace_tree: Option<TreeNode>,
}

#[derive(Clone)]
pub enum ListItem {
    SectionHeader(String),
    Separator,
    SessionItem(Session),
    TreeDir {
        name: String,
        prefix: String,
        path: PathBuf,
        folded: bool,
        descendant_repos: usize,
        descendant_open: usize,
    },
    TreeRepo {
        name: String,
        path: PathBuf,
        pills: Vec<Session>,
        active_idx: usize,
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
    pub workspace_dir: Option<PathBuf>,
    pub workspace_tree: Option<TreeNode>,
    pub display_items: Vec<ListItem>,
    pub selectable_indices: Vec<usize>,
    pub pin_target: Option<String>,
    pub constant_target: Option<String>,
    pub kill_session_info: Option<(String, String)>,
    pub pre_search_selected: usize,
    pub search_filter_active: bool,
    /// session name -> unix timestamp of last attach
    pub history: HashMap<String, u64>,
    pub filter_opened: bool,
    /// pinned session/repo names — always shown at top
    pub pins: HashSet<String>,
    /// constant session/repo names — always shown above pinned, order preserved
    pub constants: Vec<String>,
    pub table_data_y: u16,
    pub table_data_end_y: u16,
    pub table_scroll_offset: usize,
    pub last_click: Option<(Instant, usize)>,
    pub dir_order: Vec<String>,
    pub ordering_items: Vec<String>,
    pub ordering_selected: usize,
    /// constant name -> command to run when opened
    pub constant_commands: HashMap<String, String>,
    /// companion session name -> short label (e.g. "repo-2" -> "api")
    pub companion_labels: HashMap<String, String>,
    /// session name -> git worktree info (companions in git repos)
    pub worktrees: HashMap<String, WorktreeInfo>,
    /// absolute paths of folded tree directories
    pub folded_dirs: HashSet<String>,
    /// repo name -> active pill index (ephemeral, per process)
    pub repo_active_idx: HashMap<String, usize>,
    /// previously-attached session name (for jump-to-last / backtick)
    pub last_attached: Option<String>,
    /// most-recently-attached session name (shifts into last_attached on next attach)
    pub current_attached: Option<String>,
    /// pending data for the "label-before-create" flow
    pub pending_create: Option<(String, Option<PathBuf>)>,
    /// session name currently being re-labeled
    pub editing_label_target: Option<String>,
    /// sessions to restore on startup, loaded before refresh_sessions overwrites the file
    sessions_to_restore: Vec<(String, Option<PathBuf>)>,
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
            workspace_dir: workspace,
            workspace_tree: None,
            display_items: Vec::new(),
            selectable_indices: Vec::new(),
            pin_target: None,
            constant_target: None,
            kill_session_info: None,
            pre_search_selected: 0,
            search_filter_active: true,
            history: load_history(),
            filter_opened: false,
            pins: load_pins(),
            constants: load_constants(),
            table_data_y: 0,
            table_data_end_y: 0,
            table_scroll_offset: 0,
            last_click: None,
            dir_order: load_dir_order(),
            ordering_items: Vec::new(),
            ordering_selected: 0,
            constant_commands: load_constant_commands(),
            companion_labels: load_companion_labels(),
            worktrees: load_worktrees(),
            folded_dirs: load_folded_dirs(),
            repo_active_idx: HashMap::new(),
            last_attached: None,
            current_attached: None,
            pending_create: None,
            editing_label_target: None,
            sessions_to_restore: load_saved_sessions(),
        }
    }

    pub fn refresh_sessions(&mut self) {
        let dir = self.workspace_dir.clone();
        let dir_order = self.dir_order.clone();
        let (sessions, workspace_tree) = std::thread::scope(|s| {
            let sessions_h = s.spawn(|| screen::list_sessions());
            let tree_h = s.spawn(move || {
                dir.as_ref().map(|d| {
                    let mut tree = workspace::scan_tree(d);
                    if !dir_order.is_empty() {
                        reorder_tree_children(&mut tree, &dir_order);
                    }
                    tree
                })
            });
            let sessions = sessions_h.join().unwrap_or(Ok(Vec::new()));
            let tree = tree_h.join().unwrap_or(None);
            (sessions, tree)
        });
        match sessions {
            Ok(s) => self.all_sessions = s,
            Err(e) => self.set_status(format!("Error: {e}")),
        }
        if workspace_tree.is_some() {
            self.workspace_tree = workspace_tree;
        }
        save_sessions(&self.all_sessions, &self.workspace_tree);
        self.apply_search_filter();
    }

    /// Apply a completed background refresh to app state.
    pub fn apply_refresh_data(&mut self, data: RefreshData) {
        self.all_sessions = data.sessions;
        if data.workspace_tree.is_some() {
            self.workspace_tree = data.workspace_tree;
        }
        save_sessions(&self.all_sessions, &self.workspace_tree);
        self.apply_search_filter();
    }

    pub fn restore_sessions(&mut self) {
        let saved = std::mem::take(&mut self.sessions_to_restore);
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
            // If this session was a companion with a worktree, restore it at
            // the worktree path so git ops keep working. Fall back to saved
            // path (repo cwd) if the worktree is gone.
            let effective_dir = self
                .worktrees
                .get(name)
                .filter(|wt| wt.worktree_path.exists())
                .map(|wt| wt.worktree_path.clone())
                .or_else(|| path.clone());
            let result = if let Some(dir) = effective_dir {
                screen::create_session_in_dir(name, &dir)
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
                if let Some(pos) = self.constants.iter().position(|n| n == &name) {
                    self.constants.remove(pos);
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
            if let Some(pos) = self.constants.iter().position(|n| n == &name) {
                self.constants.remove(pos);
                self.set_status(format!("Removed from constants '{name}'"));
            } else {
                self.constants.push(name.clone());
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
                    &self.folded_dirs,
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

        // Inject active-idx per repo row so the UI knows which pill is current
        for item in ws_items.iter_mut() {
            if let ListItem::TreeRepo { name, pills, active_idx, .. } = item {
                let want = self.repo_active_idx.get(name).copied().unwrap_or(0);
                *active_idx = want.min(pills.len().saturating_sub(1));
            }
        }

        // Orphan sessions: not merged into any tree repo
        let mut all_orphan_sessions: Vec<&Session> = sessions_clone
            .iter()
            .filter(|s| !merged_sessions.contains(&s.name))
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

        fn extract_section(
            name_set: &HashSet<String>,
            ws_items: &[ListItem],
            orphan_items: &[ListItem],
        ) -> (Vec<ListItem>, Vec<usize>, HashSet<usize>, HashSet<usize>) {
            extract_ordered_section(None, name_set, ws_items, orphan_items, true)
        }

        fn extract_ordered_section(
            order: Option<&[String]>,
            name_set: &HashSet<String>,
            ws_items: &[ListItem],
            orphan_items: &[ListItem],
            show_dirs: bool,
        ) -> (Vec<ListItem>, Vec<usize>, HashSet<usize>, HashSet<usize>) {
            let mut ws_remove: HashSet<usize> = HashSet::new();
            let mut orphan_remove: HashSet<usize> = HashSet::new();

            let mut ws_by_name: HashMap<String, (usize, ListItem, Option<(usize, ListItem)>)> = HashMap::new();
            let mut last_dir_idx: Option<usize> = None;
            for (i, item) in ws_items.iter().enumerate() {
                match item {
                    ListItem::TreeDir { .. } | ListItem::SectionHeader(_) => {
                        last_dir_idx = Some(i);
                    }
                    ListItem::TreeRepo { name, .. } => {
                        if name_set.contains(name.as_str()) {
                            ws_remove.insert(i);
                            let dir = last_dir_idx.map(|di| (di, ws_items[di].clone()));
                            ws_by_name.insert(name.clone(), (i, item.clone(), dir));
                        }
                    }
                    _ => {}
                }
            }

            let mut orphan_by_name: HashMap<String, (usize, ListItem)> = HashMap::new();
            for (i, item) in orphan_items.iter().enumerate() {
                if let ListItem::SessionItem(session) = item {
                    if name_set.contains(session.name.as_str()) {
                        orphan_remove.insert(i);
                        orphan_by_name.insert(session.name.clone(), (i, item.clone()));
                    }
                }
            }

            let mut items: Vec<ListItem> = Vec::new();
            let mut selectable: Vec<usize> = Vec::new();
            let mut last_dir: Option<usize> = None;

            let default_order: Vec<String>;
            let iteration_order: &[String] = if let Some(o) = order {
                o
            } else {
                let mut sorted: Vec<String> = name_set.iter().cloned().collect();
                sorted.sort_by(|a, b| {
                    let da = ws_by_name.get(a).and_then(|(_, _, d)| d.as_ref().map(|(di, _)| *di)).unwrap_or(usize::MAX);
                    let db = ws_by_name.get(b).and_then(|(_, _, d)| d.as_ref().map(|(di, _)| *di)).unwrap_or(usize::MAX);
                    da.cmp(&db).then(
                        ws_by_name.get(a).map(|(wi, _, _)| *wi).unwrap_or(usize::MAX)
                            .cmp(&ws_by_name.get(b).map(|(wi, _, _)| *wi).unwrap_or(usize::MAX))
                    )
                });
                default_order = sorted;
                &default_order
            };

            for name in iteration_order {
                if let Some((_wi, repo_item, dir)) = ws_by_name.remove(name) {
                    if show_dirs {
                        if let Some((di, dir_item)) = dir {
                            if last_dir != Some(di) {
                                items.push(dir_item);
                                last_dir = Some(di);
                            }
                        }
                    }
                    selectable.push(items.len());
                    items.push(repo_item);
                } else if let Some((_oi, session_item)) = orphan_by_name.remove(name) {
                    selectable.push(items.len());
                    items.push(session_item);
                } else if order.is_some() {
                    selectable.push(items.len());
                    items.push(ListItem::SessionItem(crate::screen::Session {
                        name: name.clone(),
                        pid_name: String::new(),
                        state: crate::screen::SessionState::Detached,
                        created: None,
                        idle_secs: None,
                    }));
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

        let const_set: HashSet<String> = self.constants.iter().cloned().collect();
        let (const_items, const_selectable, const_ws_remove, const_orphan_remove) =
            extract_ordered_section(Some(&self.constants), &const_set, &ws_items, &orphan_items, false);

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
                    ListItem::TreeRepo { name, pills, .. } => {
                        if !pills.is_empty() && history.contains_key(name.as_str()) {
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
                if session.pid_name.is_empty() {
                    self.action = Action::Create(session.name, None);
                } else {
                    self.action = Action::Attach(session.pid_name);
                }
            }
            ListItem::TreeRepo { name, path, pills, active_idx, .. } => {
                if let Some(session) = pills.get(active_idx).cloned() {
                    self.record_opened(&session.name);
                    self.action = Action::Attach(session.pid_name);
                } else {
                    // Fresh pill — require a label before creating.
                    self.pending_create = Some((name, Some(path)));
                    self.create_input.clear();
                    self.cursor_pos = 0;
                    self.mode = Mode::LabelNewCompanion;
                }
            }
            ListItem::TreeDir { path, folded, .. } => {
                self.toggle_fold_dir(&path, !folded);
            }
            ListItem::SectionHeader(_) | ListItem::Separator => {}
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
            self.set_status("Name required".to_string());
            return;
        }
        match screen::create_session(&name) {
            Ok(()) => {
                self.set_status(format!("Created session '{name}'"));
                self.refresh_sessions();
                self.mode = Mode::Normal;
            }
            Err(e) => {
                self.set_status(format!("Error: {e}"));
            }
        }
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

    fn next_companion_name(base: &str, all_sessions: &[Session]) -> Option<String> {
        for n in 2..=9 {
            let candidate = format!("{base}-{n}");
            if !all_sessions.iter().any(|s| s.name == candidate) {
                return Some(candidate);
            }
        }
        None
    }

    fn companion_base_name(name: &str) -> &str {
        for n in (2..=9).rev() {
            let suffix_len = if n >= 10 { 3 } else { 2 };
            if name.len() > suffix_len {
                let (base, tail) = name.split_at(name.len() - suffix_len);
                if tail == format!("-{n}") {
                    return base;
                }
            }
        }
        name
    }

    pub fn duplicate_session(&mut self) {
        let item = match self.selected_display_item() {
            Some(item) => item.clone(),
            None => return,
        };
        match item {
            ListItem::TreeRepo { name, path, .. } => {
                if let Some(dup_name) = Self::next_companion_name(&name, &self.all_sessions) {
                    self.pending_create = Some((dup_name, Some(path)));
                    self.create_input.clear();
                    self.cursor_pos = 0;
                    self.mode = Mode::LabelNewCompanion;
                }
            }
            ListItem::SessionItem(session) => {
                let base = Self::companion_base_name(&session.name).to_string();
                if let Some(dup_name) = Self::next_companion_name(&base, &self.all_sessions) {
                    if let Some(cwd) = screen::get_session_cwd(&session.pid_name) {
                        self.pending_create = Some((dup_name, Some(cwd)));
                        self.create_input.clear();
                        self.cursor_pos = 0;
                        self.mode = Mode::LabelNewCompanion;
                    } else {
                        self.set_status("Could not determine session directory".to_string());
                    }
                }
            }
            _ => {}
        }
    }

    pub fn confirm_label_new_companion(&mut self) {
        let label = self.create_input.trim().to_string();
        if label.is_empty() {
            self.set_status("Name required".to_string());
            return;
        }
        if let Some((name, dir)) = self.pending_create.take() {
            self.companion_labels.insert(name.clone(), label);
            save_companion_labels(&self.companion_labels);
            self.record_opened(&name);
            let base = Self::companion_base_name(&name).to_string();
            let pos = Self::companion_index(&name);
            self.repo_active_idx.insert(base, pos);
            self.action = Action::Create(name, dir);
        }
        self.create_input.clear();
        self.cursor_pos = 0;
        self.mode = Mode::Normal;
    }

    pub fn cancel_label_new_companion(&mut self) {
        self.pending_create = None;
        self.create_input.clear();
        self.cursor_pos = 0;
        self.mode = Mode::Normal;
    }

    /// For a session name like "repo-3", returns 2 (0-indexed pill). Base returns 0.
    fn companion_index(name: &str) -> usize {
        for n in 2..=9 {
            let suffix = format!("-{n}");
            if name.ends_with(&suffix) {
                return n - 1;
            }
        }
        0
    }

    pub fn start_label_edit(&mut self) {
        let item = match self.selected_display_item() {
            Some(i) => i.clone(),
            None => return,
        };
        let pill_name = match item {
            ListItem::TreeRepo { pills, active_idx, .. } => {
                pills.get(active_idx).map(|s| s.name.clone())
            }
            ListItem::SessionItem(s) => Some(s.name.clone()),
            _ => None,
        };
        if let Some(name) = pill_name {
            self.create_input = self.companion_labels.get(&name).cloned().unwrap_or_default();
            self.cursor_pos = self.create_input.chars().count();
            self.editing_label_target = Some(name);
            self.mode = Mode::EditingLabel;
        }
    }

    pub fn confirm_label_edit(&mut self) {
        if let Some(name) = self.editing_label_target.take() {
            let label = self.create_input.trim().to_string();
            if label.is_empty() {
                self.companion_labels.remove(&name);
            } else {
                self.companion_labels.insert(name, label);
            }
            save_companion_labels(&self.companion_labels);
        }
        self.create_input.clear();
        self.cursor_pos = 0;
        self.mode = Mode::Normal;
    }

    pub fn cancel_label_edit(&mut self) {
        self.editing_label_target = None;
        self.create_input.clear();
        self.cursor_pos = 0;
        self.mode = Mode::Normal;
    }

    pub fn cycle_companion(&mut self, forward: bool) {
        let item = match self.selected_display_item() {
            Some(i) => i.clone(),
            None => return,
        };
        let ListItem::TreeRepo { name, path, pills, active_idx, .. } = item else {
            return;
        };
        if pills.is_empty() {
            self.record_opened(&name);
            self.action = Action::Create(name, Some(path));
            return;
        }

        if forward {
            if active_idx + 1 < pills.len() {
                let new_idx = active_idx + 1;
                self.repo_active_idx.insert(name.clone(), new_idx);
                let sess = &pills[new_idx];
                self.record_opened(&sess.name);
                self.action = Action::Attach(sess.pid_name.clone());
            } else if let Some(dup_name) = Self::next_companion_name(&name, &self.all_sessions) {
                let new_idx = Self::companion_index(&dup_name);
                self.repo_active_idx.insert(name.clone(), new_idx);
                self.record_opened(&dup_name);
                self.action = Action::Create(dup_name, Some(path));
            } else {
                self.repo_active_idx.insert(name.clone(), 0);
                let sess = &pills[0];
                self.record_opened(&sess.name);
                self.action = Action::Attach(sess.pid_name.clone());
            }
        } else {
            let new_idx = if active_idx == 0 {
                pills.len() - 1
            } else {
                active_idx - 1
            };
            self.repo_active_idx.insert(name.clone(), new_idx);
            let sess = &pills[new_idx];
            self.record_opened(&sess.name);
            self.action = Action::Attach(sess.pid_name.clone());
        }
    }

    /// Move active pill left/right without opening — pure navigation.
    pub fn shift_pill(&mut self, forward: bool) {
        let item = match self.selected_display_item() {
            Some(i) => i.clone(),
            None => return,
        };
        let ListItem::TreeRepo { name, pills, active_idx, .. } = item else {
            return;
        };
        if pills.len() <= 1 {
            return;
        }
        let new_idx = if forward {
            (active_idx + 1) % pills.len()
        } else if active_idx == 0 {
            pills.len() - 1
        } else {
            active_idx - 1
        };
        self.repo_active_idx.insert(name, new_idx);
        self.rebuild_display_list();
    }

    pub fn toggle_fold_dir(&mut self, path: &PathBuf, fold: bool) {
        let key = path.display().to_string();
        if fold {
            self.folded_dirs.insert(key);
        } else {
            self.folded_dirs.remove(&key);
        }
        save_folded_dirs(&self.folded_dirs);
        self.rebuild_display_list();
    }

    pub fn fold_at_selection(&mut self, fold: bool) {
        let item = match self.selected_display_item() {
            Some(i) => i.clone(),
            None => return,
        };
        if let ListItem::TreeDir { path, .. } = item {
            self.toggle_fold_dir(&path, fold);
        }
    }

    pub fn fold_all(&mut self) {
        if let Some(ref tree) = self.workspace_tree {
            let mut to_fold: Vec<PathBuf> = Vec::new();
            collect_dir_paths(tree, &mut to_fold);
            for p in to_fold {
                self.folded_dirs.insert(p.display().to_string());
            }
            save_folded_dirs(&self.folded_dirs);
            self.rebuild_display_list();
        }
    }

    pub fn unfold_all(&mut self) {
        self.folded_dirs.clear();
        save_folded_dirs(&self.folded_dirs);
        self.rebuild_display_list();
    }

    pub fn jump_to_last(&mut self) {
        let name = match self.last_attached.clone() {
            Some(n) => n,
            None => {
                self.set_status("No previous session".to_string());
                return;
            }
        };
        if let Some(session) = self.all_sessions.iter().find(|s| s.name == name).cloned() {
            self.record_opened(&session.name);
            self.action = Action::Attach(session.pid_name);
        } else {
            self.set_status(format!("'{name}' no longer exists"));
        }
    }

    pub fn on_tree_dir(&self) -> bool {
        matches!(self.selected_display_item(), Some(ListItem::TreeDir { .. }))
    }

    /// Decide the effective cwd for a freshly-created session. For pill-N in
    /// a git repo, this creates a worktree under `<repo>.worktrees/<name>`
    /// on branch `scrn/<name>` and records the mapping. Pill 1 (session
    /// name == repo basename) and non-git dirs use the path as-is.
    pub fn prepare_cwd_for_create(&mut self, name: &str, maybe_dir: Option<&std::path::Path>) -> Option<PathBuf> {
        let dir = maybe_dir?;
        let basename = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if name == basename || !git::is_repo(dir) {
            return Some(dir.to_path_buf());
        }

        let parent = dir.parent().unwrap_or(dir);
        let wt_root = parent.join(format!("{basename}.worktrees"));
        let _ = std::fs::create_dir_all(&wt_root);
        let wt_path = wt_root.join(name);
        if wt_path.exists() {
            // Reuse an existing worktree directory if it's already recorded.
            if let Some(info) = self.worktrees.get(name) {
                return Some(info.worktree_path.clone());
            }
            // Otherwise fall back to repo cwd — don't clobber unknown state.
            self.set_status(format!("Worktree path exists; using repo cwd for '{name}'"));
            return Some(dir.to_path_buf());
        }

        match git::create_worktree(dir, &wt_path) {
            Ok(()) => {
                self.worktrees.insert(
                    name.to_string(),
                    WorktreeInfo {
                        repo_path: dir.to_path_buf(),
                        worktree_path: wt_path.clone(),
                    },
                );
                save_worktrees(&self.worktrees);
                Some(wt_path)
            }
            Err(e) => {
                self.set_status(format!("Worktree: {e}"));
                Some(dir.to_path_buf())
            }
        }
    }

    /// Clean up a worktree for a session after it's been killed. Silent on
    /// success. If the worktree has uncommitted changes, keep it on disk
    /// and drop the mapping (user can rescue manually).
    pub fn cleanup_worktree_for(&mut self, name: &str) {
        let Some(info) = self.worktrees.remove(name) else { return };
        save_worktrees(&self.worktrees);

        if !info.worktree_path.exists() {
            return;
        }
        if git::is_worktree_dirty(&info.worktree_path) {
            self.set_status(format!(
                "Worktree kept at {} (uncommitted changes)",
                info.worktree_path.display()
            ));
            return;
        }
        let _ = git::remove_worktree(&info.worktree_path, false);
    }

    pub fn start_ordering(&mut self) {
        if let Some(ref tree) = self.workspace_tree {
            let dirs: Vec<String> = tree.children.iter()
                .filter(|c| !c.is_repo)
                .map(|c| c.name.clone())
                .collect();
            if dirs.is_empty() {
                return;
            }
            self.ordering_items = dirs;
            self.ordering_selected = 0;
            self.mode = Mode::Ordering;
        }
    }

    pub fn confirm_ordering(&mut self) {
        self.dir_order = self.ordering_items.clone();
        save_dir_order(&self.dir_order);
        if let Some(ref mut tree) = self.workspace_tree {
            reorder_tree_children(tree, &self.dir_order);
        }
        self.rebuild_display_list();
        self.mode = Mode::Normal;
    }

    pub fn cancel_ordering(&mut self) {
        self.mode = Mode::Normal;
    }

    pub fn start_constant_ordering(&mut self) {
        if self.constants.is_empty() {
            return;
        }
        self.ordering_items = self.constants.clone();
        self.ordering_selected = 0;
        self.mode = Mode::ConstantOrdering;
    }

    pub fn confirm_constant_ordering(&mut self) {
        self.constants = self.ordering_items.clone();
        save_constants(&self.constants);
        self.rebuild_display_list();
        self.mode = Mode::Normal;
    }

    pub fn cancel_constant_ordering(&mut self) {
        self.mode = Mode::Normal;
    }

    pub fn select_constant(&mut self, n: usize) {
        if n == 0 || n > self.constants.len() {
            return;
        }
        let target_name = &self.constants[n - 1];
        for (sel_idx, &disp_idx) in self.selectable_indices.iter().enumerate() {
            let name = match self.display_items.get(disp_idx) {
                Some(ListItem::TreeRepo { name, .. }) => name,
                Some(ListItem::SessionItem(s)) => &s.name,
                _ => continue,
            };
            if name == target_name {
                self.selected = sel_idx;
                self.select_for_attach();
                return;
            }
        }
    }

    pub fn constant_command(&self, session_name: &str) -> Option<&str> {
        if self.constants.contains(&session_name.to_string()) {
            self.constant_commands.get(session_name).map(|s| s.as_str())
        } else {
            None
        }
    }

    pub fn selected_item_name(&self) -> Option<String> {
        let visual_idx = *self.selectable_indices.get(self.selected)?;
        match self.display_items.get(visual_idx)? {
            ListItem::TreeRepo { name, .. } => Some(name.clone()),
            ListItem::SessionItem(s) => Some(s.name.clone()),
            _ => None,
        }
    }

    pub fn start_command_edit(&mut self) {
        if let Some(name) = self.selected_item_name() {
            if !self.constants.contains(&name) {
                return;
            }
            self.create_input = self.constant_commands.get(&name).cloned().unwrap_or_default();
            self.cursor_pos = self.create_input.chars().count();
            self.mode = Mode::EditingCommand;
        }
    }

    pub fn confirm_command(&mut self) {
        if let Some(name) = self.selected_item_name() {
            if self.create_input.is_empty() {
                self.constant_commands.remove(&name);
            } else {
                self.constant_commands.insert(name, self.create_input.clone());
            }
            save_constant_commands(&self.constant_commands);
        }
        self.create_input.clear();
        self.cursor_pos = 0;
        self.mode = Mode::Normal;
    }

    pub fn cancel_command(&mut self) {
        self.create_input.clear();
        self.cursor_pos = 0;
        self.mode = Mode::Normal;
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

    pub fn start_kill(&mut self) {
        let info = match self.selected_display_item() {
            Some(ListItem::SessionItem(session)) => {
                Some((session.name.clone(), session.pid_name.clone()))
            }
            Some(ListItem::TreeRepo { pills, active_idx, .. }) if !pills.is_empty() => {
                let s = &pills[(*active_idx).min(pills.len() - 1)];
                Some((s.name.clone(), s.pid_name.clone()))
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
                    self.cleanup_worktree_for(&name);
                    // Only overwrite status if cleanup_worktree_for didn't set one.
                    if !self.status_msg.starts_with("Worktree") {
                        self.set_status(format!("Killed '{name}'"));
                    }
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
                Ok(()) => {
                    self.cleanup_worktree_for(&session.name);
                    killed += 1;
                }
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

    /// Stamp a newly-attached session. `last_attached` holds the session
    /// *before* this one so backtick can jump back to it.
    pub fn mark_attached(&mut self, name: &str) {
        if self.current_attached.as_deref() == Some(name) {
            return;
        }
        if let Some(prev) = self.current_attached.take() {
            self.last_attached = Some(prev);
        }
        self.current_attached = Some(name.to_string());
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
    folded_dirs: &HashSet<String>,
) {
    let (iter_node, dir_prefix): (&TreeNode, String) = if !node.is_repo && depth == 0 {
        let has_direct_repos = node.children.iter().any(|c| c.is_repo);
        if has_direct_repos {
            let (compact_name, leaf) = compact_dir_chain(node);
            let path_key = leaf.path.display().to_string();
            let folded = folded_dirs.contains(&path_key);
            let (descendant_repos, descendant_open) = count_repos(leaf, session_map);
            let idx = display_items.len();
            display_items.push(ListItem::TreeDir {
                name: compact_name,
                prefix: String::new(),
                path: leaf.path.clone(),
                folded,
                descendant_repos,
                descendant_open,
            });
            selectable_indices.push(idx);
            if folded {
                return;
            }
            (leaf, String::new())
        } else {
            (node, format!("{}/", node.name))
        }
    } else {
        (node, String::new())
    };

    for child in iter_node.children.iter() {
        if child.is_repo {
            let mut pills: Vec<Session> = Vec::new();
            if let Some(s) = session_map.get(child.name.as_str()).cloned().cloned() {
                merged.insert(s.name.clone());
                pills.push(s);
            }
            for n in 2..=9 {
                let cname = format!("{}-{}", child.name, n);
                if let Some(cs) = session_map.get(cname.as_str()).cloned().cloned() {
                    merged.insert(cname);
                    pills.push(cs);
                }
            }

            let idx = display_items.len();
            display_items.push(ListItem::TreeRepo {
                name: child.name.clone(),
                path: child.path.clone(),
                pills,
                active_idx: 0,
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
            let path_key = leaf.path.display().to_string();
            let folded = folded_dirs.contains(&path_key);
            let (descendant_repos, descendant_open) = count_repos(leaf, session_map);
            let idx = display_items.len();
            display_items.push(ListItem::TreeDir {
                name: full_name,
                prefix: String::new(),
                path: leaf.path.clone(),
                folded,
                descendant_repos,
                descendant_open,
            });
            selectable_indices.push(idx);
            if !folded {
                guide_lines.push(false);
                flatten_tree(leaf, depth + 1, session_map, merged, display_items, selectable_indices, guide_lines, folded_dirs);
                guide_lines.pop();
            }
        }
    }
}

fn count_repos(
    node: &TreeNode,
    session_map: &std::collections::HashMap<&str, &Session>,
) -> (usize, usize) {
    let mut repos = 0usize;
    let mut open = 0usize;
    for child in &node.children {
        if child.is_repo {
            repos += 1;
            if session_map.contains_key(child.name.as_str()) {
                open += 1;
            }
        } else {
            let (r, o) = count_repos(child, session_map);
            repos += r;
            open += o;
        }
    }
    (repos, open)
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
            let (descendant_repos, descendant_open) = count_repos(leaf, session_map);
            let idx = display_items.len();
            display_items.push(ListItem::TreeDir {
                name: compact_name,
                prefix: String::new(),
                path: leaf.path.clone(),
                folded: false,
                descendant_repos,
                descendant_open,
            });
            selectable_indices.push(idx);
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
            let mut pills: Vec<Session> = Vec::new();
            if let Some(s) = session_map.get(child.name.as_str()).cloned().cloned() {
                merged.insert(s.name.clone());
                pills.push(s);
            }
            for n in 2..=9 {
                let cname = format!("{}-{}", child.name, n);
                if let Some(cs) = session_map.get(cname.as_str()).cloned().cloned() {
                    merged.insert(cname);
                    pills.push(cs);
                }
            }

            let idx = display_items.len();
            display_items.push(ListItem::TreeRepo {
                name: child.name.clone(),
                path: child.path.clone(),
                pills,
                active_idx: 0,
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
            let (descendant_repos, descendant_open) = count_repos(leaf, session_map);
            let idx = display_items.len();
            display_items.push(ListItem::TreeDir {
                name: full_name,
                prefix: String::new(),
                path: leaf.path.clone(),
                folded: false,
                descendant_repos,
                descendant_open,
            });
            selectable_indices.push(idx);
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

fn load_constants() -> Vec<String> {
    let path = constants_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    contents
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

fn save_constants(constants: &[String]) {
    let path = constants_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let lines: Vec<&str> = constants.iter().map(|s| s.as_str()).collect();
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

fn constant_commands_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("scrn")
        .join("constant_commands")
}

fn load_constant_commands() -> HashMap<String, String> {
    let path = constant_commands_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let mut map = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, cmd)) = line.split_once('=') {
            let name = name.trim();
            let cmd = cmd.trim();
            if !name.is_empty() && !cmd.is_empty() {
                map.insert(name.to_string(), cmd.to_string());
            }
        }
    }
    map
}

fn save_constant_commands(commands: &HashMap<String, String>) {
    let path = constant_commands_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<String> = commands.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
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
        .filter(|s| !(2..=9).any(|n| s.name.ends_with(&format!("-{n}"))))
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

fn dir_order_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("scrn").join("dir_order")
}

fn load_dir_order() -> Vec<String> {
    let path = dir_order_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    contents.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect()
}

fn save_dir_order(order: &[String]) {
    let path = dir_order_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, order.join("\n") + "\n");
}

fn companion_labels_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("scrn").join("companion_labels")
}

fn load_companion_labels() -> HashMap<String, String> {
    let path = companion_labels_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    contents
        .lines()
        .filter_map(|l| {
            let (name, label) = l.split_once('\t')?;
            if name.is_empty() || label.is_empty() { return None; }
            Some((name.to_string(), label.to_string()))
        })
        .collect()
}

fn save_companion_labels(labels: &HashMap<String, String>) {
    let path = companion_labels_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<String> = labels
        .iter()
        .map(|(name, label)| format!("{name}\t{label}"))
        .collect();
    lines.sort();
    let content = if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" };
    let _ = std::fs::write(&path, content);
}

fn worktrees_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("scrn").join("worktrees")
}

fn load_worktrees() -> HashMap<String, WorktreeInfo> {
    let path = worktrees_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let mut map = HashMap::new();
    for line in contents.lines() {
        if line.is_empty() { continue; }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 { continue; }
        let name = parts[0];
        let repo = PathBuf::from(parts[1]);
        let wt = PathBuf::from(parts[2]);
        if !wt.exists() {
            continue;
        }
        map.insert(name.to_string(), WorktreeInfo {
            repo_path: repo,
            worktree_path: wt,
        });
    }
    map
}

fn save_worktrees(worktrees: &HashMap<String, WorktreeInfo>) {
    let path = worktrees_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<String> = worktrees
        .iter()
        .map(|(name, info)| {
            format!(
                "{}\t{}\t{}",
                name,
                info.repo_path.display(),
                info.worktree_path.display()
            )
        })
        .collect();
    lines.sort();
    let content = if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" };
    let _ = std::fs::write(&path, content);
}

fn folded_dirs_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("scrn").join("folded_dirs")
}

fn load_folded_dirs() -> HashSet<String> {
    let path = folded_dirs_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    contents.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect()
}

fn save_folded_dirs(dirs: &HashSet<String>) {
    let path = folded_dirs_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<String> = dirs.iter().cloned().collect();
    lines.sort();
    let content = if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" };
    let _ = std::fs::write(&path, content);
}


fn collect_dir_paths(node: &TreeNode, out: &mut Vec<PathBuf>) {
    if !node.is_repo {
        out.push(node.path.clone());
        for child in &node.children {
            collect_dir_paths(child, out);
        }
    }
}

/// Spawn a background thread that runs `screen -ls`, `ps`, and workspace scan
/// in parallel. Returns a receiver for the completed `RefreshData`.
/// The UI can start immediately with stale data and apply the update on arrival.
pub fn spawn_refresh(
    workspace_dir: Option<PathBuf>,
    dir_order: Vec<String>,
) -> std::sync::mpsc::Receiver<RefreshData> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let (sessions, workspace_tree) = std::thread::scope(|s| {
            let sessions_h = s.spawn(|| screen::list_sessions().unwrap_or_default());
            let tree_h = s.spawn(move || {
                workspace_dir.as_ref().map(|d| {
                    let mut tree = workspace::scan_tree(d);
                    if !dir_order.is_empty() {
                        reorder_tree_children(&mut tree, &dir_order);
                    }
                    tree
                })
            });
            let sessions = sessions_h.join().unwrap_or_default();
            let tree = tree_h.join().unwrap_or(None);
            (sessions, tree)
        });
        let _ = tx.send(RefreshData {
            sessions,
            workspace_tree,
        });
    });
    rx
}

fn reorder_tree_children(tree: &mut TreeNode, order: &[String]) {
    let order_map: HashMap<&str, usize> = order.iter().enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    tree.children.sort_by(|a, b| {
        let pa = order_map.get(a.name.as_str()).copied().unwrap_or(usize::MAX);
        let pb = order_map.get(b.name.as_str()).copied().unwrap_or(usize::MAX);
        pa.cmp(&pb).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
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

