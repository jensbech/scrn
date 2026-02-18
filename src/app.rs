use std::path::PathBuf;
use std::time::Instant;

use crate::pty::PtySession;
use crate::screen::{self, Session};
use crate::workspace::{self, RepoEntry};

#[derive(PartialEq)]
pub enum Mode {
    Normal,
    Searching,
    Creating,
    Renaming,
    ConfirmKill,
    Attached,
}

pub enum Action {
    GoHome,
    None,
}

#[derive(Clone)]
pub enum ListItem {
    SectionHeader(String),
    SessionItem(Session),
    RepoItem(RepoEntry),
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
    pub attached_name: String,
    pub rename_pid_name: String,
    pub workspace_dir: Option<PathBuf>,
    pub workspace_repos: Vec<RepoEntry>,
    pub display_items: Vec<ListItem>,
    pub selectable_indices: Vec<usize>,
    pub kill_session_info: Option<(String, String)>,
}

impl App {
    pub fn new(action_file: Option<String>, workspace: Option<PathBuf>) -> Self {
        let current_session = std::env::var("STY").ok();
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
            current_session,
            action_file,
            action: Action::None,
            show_legend: false,
            pty_session: None,
            attached_name: String::new(),
            rename_pid_name: String::new(),
            workspace_dir: workspace,
            workspace_repos: Vec::new(),
            display_items: Vec::new(),
            selectable_indices: Vec::new(),
            kill_session_info: None,
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
            self.workspace_repos = workspace::scan_repos(dir);
        }
        self.apply_search_filter();
    }

    pub fn set_status(&mut self, msg: String) {
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

        // Filter repos: exclude those whose name matches an existing session name,
        // and apply search filter
        let session_names: std::collections::HashSet<&str> =
            self.sessions.iter().map(|s| s.name.as_str()).collect();

        let filtered_repos: Vec<&RepoEntry> = self
            .workspace_repos
            .iter()
            .filter(|r| {
                if session_names.contains(r.name.as_str()) {
                    return false;
                }
                if self.search_input.is_empty() {
                    true
                } else {
                    fuzzy_match(&r.name, &self.search_input).is_some()
                }
            })
            .collect();

        let has_sessions = !self.sessions.is_empty();
        let has_repos = !filtered_repos.is_empty();
        let searching = !self.search_input.is_empty();

        if has_sessions && has_repos && !searching {
            self.display_items
                .push(ListItem::SectionHeader("Sessions".to_string()));
        }
        for session in &self.sessions {
            let idx = self.display_items.len();
            self.display_items
                .push(ListItem::SessionItem(session.clone()));
            self.selectable_indices.push(idx);
        }

        if searching {
            // Flat list when searching — no group headers
            for repo in &filtered_repos {
                let idx = self.display_items.len();
                self.display_items
                    .push(ListItem::RepoItem((*repo).clone()));
                self.selectable_indices.push(idx);
            }
        } else {
            // Group repos by their group field, emitting a header per group
            let mut current_group: Option<&str> = None;
            for repo in &filtered_repos {
                if current_group != Some(&repo.group) {
                    current_group = Some(&repo.group);
                    let header = if repo.group.is_empty() {
                        // Repos directly under workspace root
                        if has_sessions {
                            "Workspaces".to_string()
                        } else {
                            // Only show group headers when there are multiple groups
                            let has_multiple_groups = filtered_repos
                                .iter()
                                .any(|r| r.group != filtered_repos[0].group);
                            if has_multiple_groups {
                                "Workspaces".to_string()
                            } else {
                                // Single group with no sessions — skip header
                                String::new()
                            }
                        }
                    } else {
                        repo.group.clone()
                    };
                    if !header.is_empty() {
                        self.display_items
                            .push(ListItem::SectionHeader(header));
                    }
                }
                let idx = self.display_items.len();
                self.display_items
                    .push(ListItem::RepoItem((*repo).clone()));
                self.selectable_indices.push(idx);
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
        self.apply_search_filter();
        self.mode = Mode::Normal;
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
        let pty_rows = term_rows.saturating_sub(3);
        let pty_cols = term_cols.saturating_sub(2);
        let rc = crate::screen::ensure_screenrc();
        match PtySession::spawn("screen", &["-c", &rc, "-r", pid_name], pty_rows, pty_cols) {
            Ok(pty) => {
                pty.write_all(b"\x0c");
                self.pty_session = Some(pty);
                self.attached_name = name.to_string();
                self.mode = Mode::Attached;
            }
            Err(e) => {
                self.set_status(format!("Failed to attach: {e}"));
            }
        }
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
            ListItem::RepoItem(repo) => {
                let name = repo.name.clone();
                let path = repo.path.clone();
                match screen::create_session_in_dir(&name, &path) {
                    Ok(()) => {
                        self.set_status(format!("Created session '{name}'"));
                        self.refresh_sessions();
                        // Find the newly created session and attach
                        let pid_name = self
                            .sessions
                            .iter()
                            .find(|s| s.name == name)
                            .map(|s| s.pid_name.clone());
                        if let Some(pid_name) = pid_name {
                            self.attach_session(&name, &pid_name, term_rows, term_cols);
                        }
                    }
                    Err(e) => {
                        self.set_status(format!("Error: {e}"));
                    }
                }
            }
            ListItem::SectionHeader(_) => {}
        }
    }

    pub fn detach_pty(&mut self) {
        self.pty_session = None;
        self.attached_name.clear();
        self.mode = Mode::Normal;
        self.refresh_sessions();
    }

    pub fn resize_pty(&mut self, term_rows: u16, term_cols: u16) {
        if let Some(ref mut pty) = self.pty_session {
            let pty_rows = term_rows.saturating_sub(3);
            let pty_cols = term_cols.saturating_sub(2);
            pty.resize(pty_rows, pty_cols);
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
        if let Some(ListItem::SessionItem(session)) = self.selected_display_item() {
            self.kill_session_info = Some((session.name.clone(), session.pid_name.clone()));
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
}

pub fn fuzzy_match(haystack: &str, needle: &str) -> Option<Vec<usize>> {
    let haystack_lower: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    let needle_lower: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();

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
    Some(positions)
}
