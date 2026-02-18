use std::time::Instant;

use crate::pty::PtySession;
use crate::screen::{self, Session};

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
}

impl App {
    pub fn new(action_file: Option<String>) -> Self {
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
        }
    }

    pub fn refresh_sessions(&mut self) {
        match screen::list_sessions() {
            Ok(sessions) => {
                self.all_sessions = sessions;
                self.apply_search_filter();
            }
            Err(e) => {
                self.set_status(format!("Error: {e}"));
            }
        }
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
        if !self.sessions.is_empty() && self.selected < self.sessions.len() - 1 {
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
                // Hide the session we're currently inside
                if self.is_current_session(s) {
                    return false;
                }
                // Hide auto-named sessions (no explicit -S name given)
                // These have tty device names like "ttys006.hostname" or "pts/0.hostname"
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
        if self.selected >= self.sessions.len() {
            self.selected = self.sessions.len().saturating_sub(1);
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

    pub fn attach_selected(&mut self, term_rows: u16, term_cols: u16) {
        if let Some(session) = self.sessions.get(self.selected) {
            if let Some(ref current) = self.current_session {
                if *current == session.pid_name {
                    self.set_status("Already in this session".to_string());
                    return;
                }
            }
            let name = session.name.clone();
            let pid_name = session.pid_name.clone();
            // Reserve: 2 rows for block borders + 1 row for status bar, 2 cols for borders
            let pty_rows = term_rows.saturating_sub(3);
            let pty_cols = term_cols.saturating_sub(2);
            match PtySession::spawn("screen", &["-r", &pid_name], pty_rows, pty_cols) {
                Ok(pty) => {
                    // Clear shell so prompt starts at top (fixes size
                    // mismatch from sessions created with `screen -dmS`)
                    pty.write_all(b"\x0c");
                    self.pty_session = Some(pty);
                    self.attached_name = name;
                    self.mode = Mode::Attached;
                }
                Err(e) => {
                    self.set_status(format!("Failed to attach: {e}"));
                }
            }
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
            // Same border math as attach_selected
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
        if let Some(session) = self.sessions.get(self.selected) {
            self.rename_pid_name = session.pid_name.clone();
            self.create_input = session.name.clone();
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
        if !self.sessions.is_empty() {
            self.mode = Mode::ConfirmKill;
        }
    }

    pub fn confirm_kill(&mut self) {
        if let Some(session) = self.sessions.get(self.selected).cloned() {
            match screen::kill_session(&session.pid_name) {
                Ok(()) => {
                    self.set_status(format!("Killed '{}'", session.name));
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
        self.mode = Mode::Normal;
    }

    pub fn go_home(&mut self) {
        if self.current_session.is_some() {
            self.action = Action::GoHome;
        }
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.sessions.get(self.selected)
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
