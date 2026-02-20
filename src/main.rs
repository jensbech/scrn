mod app;
mod config;
mod logging;
mod pty;
mod screen;
mod shell;
mod ui;
mod workspace;

use std::fs;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags, MouseButton, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{Action, App, Mode, Pane, SidebarFocus};

/// Poll stdin and PTY file descriptors simultaneously.
/// Returns (stdin_ready, pty_ready) so the caller can drain PTY data
/// immediately instead of waiting for the crossterm poll timeout.
fn poll_fds(pty_fds: &[i32], timeout_ms: i32) -> (bool, bool) {
    let mut fds: Vec<libc::pollfd> = Vec::with_capacity(1 + pty_fds.len());
    fds.push(libc::pollfd {
        fd: 0, // STDIN_FILENO
        events: libc::POLLIN,
        revents: 0,
    });
    for &fd in pty_fds {
        fds.push(libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
    }
    let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
    if ret <= 0 {
        return (false, false);
    }
    let stdin_ready = fds[0].revents & libc::POLLIN != 0;
    let pty_ready = fds[1..].iter().any(|f| f.revents & libc::POLLIN != 0);
    (stdin_ready, pty_ready)
}

fn input_insert(s: &mut String, cursor: &mut usize, c: char) {
    let bp = s
        .char_indices()
        .nth(*cursor)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s.insert(bp, c);
    *cursor += 1;
}

fn input_backspace(s: &mut String, cursor: &mut usize) {
    if *cursor > 0 {
        *cursor -= 1;
        let bp = s
            .char_indices()
            .nth(*cursor)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        s.remove(bp);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    logging::setup_panic_hook();

    let args: Vec<String> = std::env::args().collect();

    // Handle subcommands
    match args.get(1).map(|s| s.as_str()) {
        Some("--version" | "-v") => {
            println!("scrn {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("init") => {
            let shell = args.get(2).map(|s| s.as_str()).unwrap_or("zsh");
            match shell::init_script(shell) {
                Ok(script) => {
                    print!("{script}");
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {}
    }

    // Verify GNU Screen version before starting
    if let Err(e) = screen::check_version() {
        eprintln!("scrn: {e}");
        std::process::exit(1);
    }

    // Parse flags
    let mut action_file = None;
    let mut cli_workspace = None;
    let mut sidebar_mode = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--action-file" => {
                if let Some(path) = args.get(i + 1) {
                    action_file = Some(path.clone());
                    i += 2;
                    continue;
                }
            }
            "--workspace" | "-w" => {
                if let Some(path) = args.get(i + 1) {
                    cli_workspace = Some(path.clone());
                    i += 2;
                    continue;
                }
            }
            "--sidebar" | "-s" => {
                sidebar_mode = true;
            }
            _ => {}
        }
        i += 1;
    }

    let cfg = config::Config::load(cli_workspace.as_deref());
    let sidebar = sidebar_mode || cfg.sidebar;

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    terminal.clear()?;

    let mut app = App::new(action_file, cfg.workspace, sidebar);
    app.refresh_sessions();
    app.restore_sessions();

    let exit_action;
    let mut last_esc: Option<Instant> = None;

    // Track whether PTY content needs re-rendering (dirty = new data or resize)
    let mut pty_needs_render = false;
    // Track when UI chrome (border/status bar) needs redrawing via ratatui
    let mut ui_needs_draw = false;
    // Rate-limit rendering — accumulate PTY data between frames
    let mut last_render = Instant::now();
    const FRAME_BUDGET: Duration = Duration::from_millis(8); // ~120fps max

    // Cache terminal size to avoid an ioctl syscall every frame
    let (mut term_cols, mut term_rows) = crossterm::terminal::size().unwrap_or((80, 24));

    // Previous frame screens for differential rendering — None forces full redraw
    let mut prev_screen_left: Option<vt100::Screen> = None;
    let mut prev_screen_right: Option<vt100::Screen> = None;

    // Scroll offsets (0 = live view, >0 = scrolled back into vt100 scrollback)
    let mut scroll_offset_left: usize = 0;
    let mut scroll_offset_right: usize = 0;

    // Track alternate screen state to force full redraw on mode change
    let mut alt_screen_left = false;
    let mut alt_screen_right = false;


    // Mouse text selection: (pane, selection coordinates in pane-local space)
    let mut selection: Option<(Pane, ui::PaneSelection)> = None;

    loop {
        // ── 1. Drain PTY output (fast — just memcpy + vt100 parse) ──
        if app.mode == Mode::Attached {
            let (left_dirty, should_detach) = if let Some(ref mut pty) = app.pty_session {
                let dirty = pty.try_read();
                (dirty, !pty.is_running())
            } else {
                (false, true)
            };

            let right_dirty = if let Some(ref mut pty_right) = app.pty_right {
                let dirty = pty_right.try_read();
                if !pty_right.is_running() {
                    // Right pane died — drop it and continue with left pane only
                    app.pty_right = None;
                    app.attached_right_name.clear();
                    prev_screen_right = None;
                    scroll_offset_right = 0;
                    prev_screen_left = None; // force full redraw
                    ui_needs_draw = true;
                    false
                } else {
                    dirty
                }
            } else {
                false
            };

            if should_detach {
                prev_screen_left = None;
                prev_screen_right = None;
                scroll_offset_left = 0;
                scroll_offset_right = 0;
                selection = None;
                app.detach_pty();
            }

            pty_needs_render = pty_needs_render || left_dirty || right_dirty;

            // Force full redraw when alternate screen mode changes (app enter/exit)
            if let Some(ref pty) = app.pty_session {
                let alt = pty.alternate_screen();
                if alt != alt_screen_left {
                    alt_screen_left = alt;
                    prev_screen_left = None;
                }
            }
            if let Some(ref pty) = app.pty_right {
                let alt = pty.alternate_screen();
                if alt != alt_screen_right {
                    alt_screen_right = alt;
                    prev_screen_right = None;
                }
            }

        }

        // ── 2. Render (rate-limited, skip ratatui on PTY-only frames) ──
        if app.mode == Mode::Attached {
            let render_due = last_render.elapsed() >= FRAME_BUDGET;
            if (pty_needs_render || ui_needs_draw) && render_due {
                // Begin synchronized output + hide cursor
                write!(terminal.backend_mut(), "\x1b[?2026h\x1b[?25l")?;

                // Only call ratatui draw when UI chrome needs updating
                // (first frame, resize, pane swap). Skipping this on normal
                // frames avoids buffer management + diff + 2 flushes.
                if ui_needs_draw {
                    terminal.draw(|f| ui::draw(f, &app))?;
                    ui_needs_draw = false;
                }

                // Render PTY cells + cursor directly
                let (cols, rows) = (term_cols, term_rows);

                if app.pty_right.is_some() {
                    let (left_x, left_w, right_x, right_w, inner_y, inner_h) =
                        if app.sidebar_mode {
                            ui::sidebar_two_pane_geometry(&app, cols, rows)
                        } else {
                            ui::two_pane_geometry(cols, rows)
                        };

                    if pty_needs_render {
                        // Force full redraw when text selection is active
                        if selection.is_some() {
                            prev_screen_left = None;
                            prev_screen_right = None;
                        }
                        // Render left pane
                        let sb_total_left;
                        if let Some(ref mut pty) = app.pty_session {
                            // Get vt100 parser's scrollback capacity
                            pty.set_scrollback(usize::MAX);
                            let vt_sb_left = pty.scrollback_offset();
                            pty.set_scrollback(0);
                            let hist_len = app.screen_history_left.len();
                            sb_total_left = vt_sb_left + hist_len;
                            scroll_offset_left = scroll_offset_left.min(sb_total_left);

                            if scroll_offset_left > vt_sb_left && hist_len > 0 {
                                // Scrolled past vt100 buffer — render from screen history
                                let extra = scroll_offset_left - vt_sb_left;
                                let history_line = hist_len.saturating_sub(extra);
                                ui::render_history_lines(
                                    terminal.backend_mut(),
                                    &app.screen_history_left,
                                    history_line.saturating_sub(inner_h as usize),
                                    left_x, inner_y, left_w, inner_h,
                                )?;
                                prev_screen_left = None;
                            } else if scroll_offset_left > 0 {
                                pty.set_scrollback(scroll_offset_left);
                                ui::render_pty_direct(
                                    terminal.backend_mut(),
                                    pty.screen(),
                                    None,
                                    left_x, inner_y, left_w, inner_h,
                                )?;
                                if let Some((Pane::Left, sel)) = &selection {
                                    ui::render_selection(terminal.backend_mut(), pty.screen(), sel, left_x, inner_y, left_w, inner_h)?;
                                }
                                pty.set_scrollback(0);
                                prev_screen_left = None;
                            } else {
                                ui::render_pty_direct(
                                    terminal.backend_mut(),
                                    pty.screen(),
                                    prev_screen_left.as_ref(),
                                    left_x, inner_y, left_w, inner_h,
                                )?;
                                if let Some((Pane::Left, sel)) = &selection {
                                    ui::render_selection(terminal.backend_mut(), pty.screen(), sel, left_x, inner_y, left_w, inner_h)?;
                                }
                                prev_screen_left = Some(pty.screen().clone());
                            }
                            if scroll_offset_left > 0 {
                                ui::render_scrollbar(
                                    terminal.backend_mut(), scroll_offset_left, sb_total_left,
                                    left_x, inner_y, left_w, inner_h,
                                )?;
                            }
                        }
                        // Render right pane
                        let sb_total_right;
                        if let Some(ref mut pty) = app.pty_right {
                            pty.set_scrollback(usize::MAX);
                            let vt_sb_right = pty.scrollback_offset();
                            pty.set_scrollback(0);
                            let hist_len = app.screen_history_right.len();
                            sb_total_right = vt_sb_right + hist_len;
                            scroll_offset_right = scroll_offset_right.min(sb_total_right);

                            if scroll_offset_right > vt_sb_right && hist_len > 0 {
                                let extra = scroll_offset_right - vt_sb_right;
                                let history_line = hist_len.saturating_sub(extra);
                                ui::render_history_lines(
                                    terminal.backend_mut(),
                                    &app.screen_history_right,
                                    history_line.saturating_sub(inner_h as usize),
                                    right_x, inner_y, right_w, inner_h,
                                )?;
                                prev_screen_right = None;
                            } else if scroll_offset_right > 0 {
                                pty.set_scrollback(scroll_offset_right);
                                ui::render_pty_direct(
                                    terminal.backend_mut(),
                                    pty.screen(),
                                    None,
                                    right_x, inner_y, right_w, inner_h,
                                )?;
                                if let Some((Pane::Right, sel)) = &selection {
                                    ui::render_selection(terminal.backend_mut(), pty.screen(), sel, right_x, inner_y, right_w, inner_h)?;
                                }
                                pty.set_scrollback(0);
                                prev_screen_right = None;
                            } else {
                                ui::render_pty_direct(
                                    terminal.backend_mut(),
                                    pty.screen(),
                                    prev_screen_right.as_ref(),
                                    right_x, inner_y, right_w, inner_h,
                                )?;
                                if let Some((Pane::Right, sel)) = &selection {
                                    ui::render_selection(terminal.backend_mut(), pty.screen(), sel, right_x, inner_y, right_w, inner_h)?;
                                }
                                prev_screen_right = Some(pty.screen().clone());
                            }
                            if scroll_offset_right > 0 {
                                ui::render_scrollbar(
                                    terminal.backend_mut(), scroll_offset_right, sb_total_right,
                                    right_x, inner_y, right_w, inner_h,
                                )?;
                            }
                        }
                    }

                    // Cursor for active pane (only when at live view)
                    let (active_x, active_offset, cursor_screen) = match app.active_pane {
                        Pane::Left => (
                            left_x,
                            scroll_offset_left,
                            app.pty_session.as_ref().map(|p| p.screen()),
                        ),
                        Pane::Right => (
                            right_x,
                            scroll_offset_right,
                            app.pty_right.as_ref().map(|p| p.screen()),
                        ),
                    };
                    if active_offset == 0 {
                        if let Some(screen) = cursor_screen {
                            ui::write_pty_cursor(
                                terminal.backend_mut(),
                                screen,
                                active_x,
                                inner_y,
                            )?;
                        }
                    }
                } else {
                    let (inner_x, inner_y, inner_w, inner_h) = if app.sidebar_mode {
                        ui::sidebar_geometry(&app, cols, rows)
                    } else {
                        (1u16, 1u16, cols.saturating_sub(2), rows.saturating_sub(2))
                    };

                    if pty_needs_render {
                        // Force full redraw when text selection is active
                        if selection.is_some() {
                            prev_screen_left = None;
                        }
                        let sb_total;
                        if let Some(ref mut pty) = app.pty_session {
                            pty.set_scrollback(usize::MAX);
                            let vt_sb = pty.scrollback_offset();
                            pty.set_scrollback(0);
                            let hist_len = app.screen_history_left.len();
                            sb_total = vt_sb + hist_len;
                            scroll_offset_left = scroll_offset_left.min(sb_total);

                            if scroll_offset_left > vt_sb && hist_len > 0 {
                                let extra = scroll_offset_left - vt_sb;
                                let history_line = hist_len.saturating_sub(extra);
                                ui::render_history_lines(
                                    terminal.backend_mut(),
                                    &app.screen_history_left,
                                    history_line.saturating_sub(inner_h as usize),
                                    inner_x, inner_y, inner_w, inner_h,
                                )?;
                                prev_screen_left = None;
                            } else if scroll_offset_left > 0 {
                                pty.set_scrollback(scroll_offset_left);
                                ui::render_pty_direct(
                                    terminal.backend_mut(),
                                    pty.screen(),
                                    None,
                                    inner_x, inner_y, inner_w, inner_h,
                                )?;
                                if let Some((_, sel)) = &selection {
                                    ui::render_selection(terminal.backend_mut(), pty.screen(), sel, inner_x, inner_y, inner_w, inner_h)?;
                                }
                                pty.set_scrollback(0);
                                prev_screen_left = None;
                            } else {
                                ui::render_pty_direct(
                                    terminal.backend_mut(),
                                    pty.screen(),
                                    prev_screen_left.as_ref(),
                                    inner_x, inner_y, inner_w, inner_h,
                                )?;
                                if let Some((_, sel)) = &selection {
                                    ui::render_selection(terminal.backend_mut(), pty.screen(), sel, inner_x, inner_y, inner_w, inner_h)?;
                                }
                                prev_screen_left = Some(pty.screen().clone());
                            }
                            if scroll_offset_left > 0 {
                                ui::render_scrollbar(
                                    terminal.backend_mut(), scroll_offset_left, sb_total,
                                    inner_x, inner_y, inner_w, inner_h,
                                )?;
                            }
                        }
                    }

                    // Cursor (only when at live view)
                    if scroll_offset_left == 0 {
                        if let Some(ref pty) = app.pty_session {
                            ui::write_pty_cursor(
                                terminal.backend_mut(),
                                pty.screen(),
                                inner_x,
                                inner_y,
                            )?;
                        }
                    }
                }

                // End synchronized output — terminal renders the whole frame at once
                write!(terminal.backend_mut(), "\x1b[?2026l")?;
                terminal.backend_mut().flush()?;
                pty_needs_render = false;
                last_render = Instant::now();
            }
        } else {
            terminal.draw(|f| ui::draw(f, &app))?;
        }

        // Auto-clear stale status messages (only in list view)
        if app.mode != Mode::Attached
            && !app.status_msg.is_empty()
            && app.status_set_at.elapsed() > Duration::from_secs(5)
        {
            app.status_msg.clear();
        }

        // Check if an action was set (go home)
        match &app.action {
            Action::GoHome => {
                exit_action = std::mem::replace(&mut app.action, Action::None);
                break;
            }
            Action::None => {}
        }

        // ── 3. Poll for input ──
        // In attached mode, use poll(2) to multiplex stdin + PTY master fds
        // so we wake immediately when PTY data arrives instead of waiting
        // for the crossterm poll timeout (~8ms). This cuts perceived latency
        // roughly in half for full-screen apps.
        let poll_duration = if app.mode == Mode::Attached {
            if pty_needs_render {
                FRAME_BUDGET
                    .saturating_sub(last_render.elapsed())
                    .max(Duration::from_millis(1))
            } else {
                FRAME_BUDGET
            }
        } else {
            Duration::from_millis(100)
        };

        let has_event = if app.mode == Mode::Attached {
            // Check for already-buffered crossterm events first
            if event::poll(Duration::ZERO)? {
                true
            } else {
                // Multiplex: wait for stdin OR PTY data, whichever first
                let mut pty_fds = Vec::with_capacity(2);
                if let Some(ref pty) = app.pty_session {
                    pty_fds.push(pty.master_fd());
                }
                if let Some(ref pty) = app.pty_right {
                    pty_fds.push(pty.master_fd());
                }
                let timeout_ms = poll_duration.as_millis() as i32;
                poll_fds(&pty_fds, timeout_ms);
                // If stdin got data, crossterm will see it
                event::poll(Duration::ZERO)?
            }
        } else {
            event::poll(poll_duration)?
        };

        if has_event {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.mode {
                    Mode::Attached if app.sidebar_mode && app.sidebar_focus == SidebarFocus::List => {
                        // Sidebar mode, focus on list: navigate sessions
                        match key.code {
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.move_up();
                                ui_needs_draw = true;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.move_down();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('g') => {
                                app.move_to_top();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('G') => {
                                app.move_to_bottom();
                                ui_needs_draw = true;
                            }
                            KeyCode::Enter => {
                                let (cols, rows) = (term_cols, term_rows);
                                prev_screen_left = None;
                                prev_screen_right = None;
                                scroll_offset_left = 0;
                                scroll_offset_right = 0;
                                app.sidebar_switch_session(rows, cols);
                                if app.mode == Mode::Attached {
                                    pty_needs_render = true;
                                    ui_needs_draw = true;
                                }
                            }
                            KeyCode::Tab => {
                                app.sidebar_focus = SidebarFocus::Content;
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('/') => {
                                app.start_search();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('c') => {
                                app.start_create();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('x') => {
                                app.start_kill();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('n') => {
                                app.start_rename();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('q') => {
                                app.mode = Mode::ConfirmQuit;
                                ui_needs_draw = true;
                            }
                            KeyCode::Esc => {
                                if !app.search_input.is_empty() {
                                    app.clear_search();
                                    ui_needs_draw = true;
                                } else {
                                    app.mode = Mode::ConfirmQuit;
                                    ui_needs_draw = true;
                                }
                            }
                            KeyCode::Char('p') => {
                                app.toggle_pin();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('r') => {
                                app.refresh_sessions();
                                ui_needs_draw = true;
                            }
                            KeyCode::Char('?') => {
                                app.show_legend = !app.show_legend;
                                ui_needs_draw = true;
                            }
                            _ => {}
                        }
                    }
                    Mode::Attached => {
                        // Clear text selection on any key press
                        if selection.is_some() {
                            selection = None;
                            prev_screen_left = None;
                            prev_screen_right = None;
                            pty_needs_render = true;
                        }
                        // Ctrl+E (scroll up) / Ctrl+N (scroll down) — always check first
                        let is_ctrl = key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
                        let mut handled = false;
                        if is_ctrl && matches!(key.code, KeyCode::Char('e') | KeyCode::Char('n')) {
                            let in_alt_screen = match app.active_pane {
                                Pane::Left => app.pty_session.as_ref().map_or(false, |p| p.alternate_screen()),
                                Pane::Right => app.pty_right.as_ref().map_or(false, |p| p.alternate_screen()),
                            };

                            if in_alt_screen {
                                // Full-screen app running — forward as Page Up/Down
                                let active_pty = match app.active_pane {
                                    Pane::Left => app.pty_session.as_ref(),
                                    Pane::Right => app.pty_right.as_ref(),
                                };
                                if let Some(pty) = active_pty {
                                    let seq: &[u8] = match key.code {
                                        KeyCode::Char('e') => b"\x1b[5~", // Page Up
                                        _ => b"\x1b[6~",                  // Page Down
                                    };
                                    pty.write_all(seq);
                                }
                            } else {
                                // Normal screen — scroll scrn's scrollback by 1 line
                                let offset = match app.active_pane {
                                    Pane::Left => &mut scroll_offset_left,
                                    Pane::Right => &mut scroll_offset_right,
                                };
                                match key.code {
                                    KeyCode::Char('e') => { *offset += 1; }
                                    KeyCode::Char('n') => { *offset = offset.saturating_sub(1); }
                                    _ => {}
                                }
                                pty_needs_render = true;
                            }
                            handled = true;
                        }

                        // Ctrl+O — in sidebar mode: switch focus to list; otherwise: detach
                        if !handled && is_ctrl && key.code == KeyCode::Char('o') {
                            last_esc = None;
                            if app.sidebar_mode {
                                app.sidebar_focus = SidebarFocus::List;
                                ui_needs_draw = true;
                            } else {
                                prev_screen_left = None;
                                prev_screen_right = None;
                                scroll_offset_left = 0;
                                scroll_offset_right = 0;
                                app.detach_pty();
                                selection = None;
                            }
                            handled = true;
                        }

                        // Tab in sidebar mode: switch focus to list
                        if !handled && app.sidebar_mode && key.code == KeyCode::Tab {
                            last_esc = None;
                            app.sidebar_focus = SidebarFocus::List;
                            ui_needs_draw = true;
                            handled = true;
                        }

                        // When scrolled back, intercept navigation keys
                        if !handled {
                            let active_offset = match app.active_pane {
                                Pane::Left => &mut scroll_offset_left,
                                Pane::Right => &mut scroll_offset_right,
                            };
                            if *active_offset > 0 {
                                handled = true;
                                match key.code {
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        *active_offset += 1;
                                        pty_needs_render = true;
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        *active_offset = active_offset.saturating_sub(1);
                                        pty_needs_render = true;
                                    }
                                    KeyCode::PageUp => {
                                        let page = term_rows.saturating_sub(2) as usize;
                                        *active_offset += page;
                                        pty_needs_render = true;
                                    }
                                    KeyCode::PageDown => {
                                        let page = term_rows.saturating_sub(2) as usize;
                                        *active_offset = active_offset.saturating_sub(page);
                                        pty_needs_render = true;
                                    }
                                    KeyCode::Esc => {
                                        *active_offset = 0;
                                        pty_needs_render = true;
                                    }
                                    _ => {
                                        // Snap to live, fall through to normal handling
                                        *active_offset = 0;
                                        pty_needs_render = true;
                                        handled = false;
                                    }
                                }
                            }
                        }

                        if !handled {
                            if key.code == KeyCode::Esc {
                                if last_esc
                                    .is_some_and(|t| t.elapsed() < Duration::from_millis(300))
                                {
                                    // Double Esc — detach
                                    last_esc = None;
                                    prev_screen_left = None;
                                    prev_screen_right = None;
                                    scroll_offset_left = 0;
                                    scroll_offset_right = 0;
                                    app.detach_pty();
                                    selection = None;
                                } else {
                                    // First Esc — forward to active pane PTY and start timer
                                    last_esc = Some(Instant::now());
                                    let active_pty = match app.active_pane {
                                        Pane::Left => app.pty_session.as_ref(),
                                        Pane::Right => app.pty_right.as_ref(),
                                    };
                                    if let Some(pty) = active_pty {
                                        pty.write_all(&[0x1b]);
                                    }
                                }
                            } else if key.code == KeyCode::F(6)
                                || (key.code == KeyCode::Char('s')
                                    && key
                                        .modifiers
                                        .contains(crossterm::event::KeyModifiers::CONTROL))
                            {
                                // F6 / Ctrl+S — swap active pane
                                last_esc = None;
                                app.swap_pane();
                                ui_needs_draw = true;
                                prev_screen_left = None;
                                prev_screen_right = None;
                            } else {
                                last_esc = None;
                                // Forward everything else to the active pane's PTY
                                let active_pty = match app.active_pane {
                                    Pane::Left => app.pty_session.as_ref(),
                                    Pane::Right => app.pty_right.as_ref(),
                                };
                                if let Some(pty) = active_pty {
                                    let bytes = pty::key_to_bytes(&key, pty.application_cursor());
                                    pty.write_all(&bytes);
                                }
                            }
                        }
                    }
                    Mode::Normal => match key.code {
                        KeyCode::Esc => {
                            if !app.search_input.is_empty() {
                                app.clear_search();
                            } else {
                                app.mode = Mode::ConfirmQuit;
                            }
                        }
                        KeyCode::Char('q') => {
                            app.mode = Mode::ConfirmQuit;
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                        KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                        KeyCode::Char('g') => app.move_to_top(),
                        KeyCode::Char('G') => app.move_to_bottom(),
                        KeyCode::Char('o') => app.toggle_opened_filter(),
                        KeyCode::Enter => {
                            let (cols, rows) = (term_cols, term_rows);
                            app.attach_selected(rows, cols);
                            if app.mode == Mode::Attached {
                                if app.sidebar_mode {
                                    app.sidebar_focus = SidebarFocus::Content;
                                }
                                pty_needs_render = true;
                                ui_needs_draw = true;
                                prev_screen_left = None;
                                prev_screen_right = None;
                                scroll_offset_left = 0;
                                scroll_offset_right = 0;
                            }
                        }
                        KeyCode::Char('c') => app.start_create(),
                        KeyCode::Char('n') => app.start_rename(),
                        KeyCode::Char('x') => app.start_kill(),
                        KeyCode::Char('X') => app.start_kill_all(),
                        KeyCode::Char('d') => app.go_home(),
                        KeyCode::Char('/') => app.start_search(),
                        KeyCode::Char('p') => app.toggle_pin(),
                        KeyCode::Char('r') => app.refresh_sessions(),
                        KeyCode::Char('?') => app.show_legend = !app.show_legend,
                        _ => {}
                    },
                    Mode::Searching => match key.code {
                        KeyCode::Esc => app.clear_search(),
                        KeyCode::Enter => app.confirm_search(),
                        KeyCode::Up => app.move_up(),
                        KeyCode::Down => app.move_down(),
                        KeyCode::Backspace => {
                            app.search_input.pop();
                            app.apply_search_filter();
                        }
                        KeyCode::Char(c) => {
                            app.search_input.push(c);
                            app.apply_search_filter();
                        }
                        _ => {}
                    },
                    Mode::Creating => match key.code {
                        KeyCode::Enter => app.confirm_create(),
                        KeyCode::Esc => app.cancel_create(),
                        KeyCode::Left => {
                            if app.cursor_pos > 0 {
                                app.cursor_pos -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if app.cursor_pos < app.create_input.chars().count() {
                                app.cursor_pos += 1;
                            }
                        }
                        KeyCode::Backspace => {
                            input_backspace(&mut app.create_input, &mut app.cursor_pos);
                        }
                        KeyCode::Char(c) => {
                            input_insert(&mut app.create_input, &mut app.cursor_pos, c);
                        }
                        _ => {}
                    },
                    Mode::Renaming => match key.code {
                        KeyCode::Enter => app.confirm_rename(),
                        KeyCode::Esc => app.cancel_rename(),
                        KeyCode::Left => {
                            if app.cursor_pos > 0 {
                                app.cursor_pos -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if app.cursor_pos < app.create_input.chars().count() {
                                app.cursor_pos += 1;
                            }
                        }
                        KeyCode::Backspace => {
                            input_backspace(&mut app.create_input, &mut app.cursor_pos);
                        }
                        KeyCode::Char(c) => {
                            input_insert(&mut app.create_input, &mut app.cursor_pos, c);
                        }
                        _ => {}
                    },
                    Mode::ConfirmKill => match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => app.confirm_kill(),
                        KeyCode::Char('n') | KeyCode::Esc => app.cancel_kill(),
                        _ => {}
                    },
                    Mode::ConfirmKillAll1 => match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => app.confirm_kill_all_step1(),
                        KeyCode::Char('n') | KeyCode::Esc => app.cancel_kill_all(),
                        _ => {}
                    },
                    Mode::ConfirmKillAll2 => match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => app.confirm_kill_all_step2(),
                        KeyCode::Char('n') | KeyCode::Esc => app.cancel_kill_all(),
                        _ => {}
                    },
                    Mode::ConfirmQuit => match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => {
                            exit_action = Action::None;
                            break;
                        }
                        KeyCode::Char('n') | KeyCode::Esc => {
                            // If PTY is still alive (sidebar mode), return to Attached
                            if app.pty_session.is_some() {
                                app.mode = Mode::Attached;
                            } else {
                                app.mode = Mode::Normal;
                            }
                        }
                        _ => {}
                    },
                },
                Event::Paste(text) => {
                    if app.mode == Mode::Attached {
                        let active_pty = match app.active_pane {
                            Pane::Left => app.pty_session.as_ref(),
                            Pane::Right => app.pty_right.as_ref(),
                        };
                        if let Some(pty) = active_pty {
                            if pty.screen().bracketed_paste() {
                                pty.write_all(b"\x1b[200~");
                                pty.write_all(text.as_bytes());
                                pty.write_all(b"\x1b[201~");
                            } else {
                                pty.write_all(text.as_bytes());
                            }
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    if app.mode == Mode::Attached {
                        // In sidebar mode, check if click is in sidebar area
                        let in_sidebar = app.sidebar_mode && mouse.column < app.sidebar_width(term_cols);

                        if in_sidebar {
                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    app.sidebar_focus = SidebarFocus::List;
                                    ui_needs_draw = true;
                                }
                                MouseEventKind::ScrollUp => {
                                    app.move_up();
                                    ui_needs_draw = true;
                                }
                                MouseEventKind::ScrollDown => {
                                    app.move_down();
                                    ui_needs_draw = true;
                                }
                                _ => {}
                            }
                        } else {
                            // Content area (or non-sidebar mode)
                            if app.sidebar_mode && app.sidebar_focus == SidebarFocus::List {
                                app.sidebar_focus = SidebarFocus::Content;
                                ui_needs_draw = true;
                            }
                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    if let Some((pane, row, col)) = hit_test_pane(&app, mouse.column, mouse.row, term_cols, term_rows) {
                                        selection = Some((pane, ui::PaneSelection {
                                            start_row: row,
                                            start_col: col,
                                            end_row: row,
                                            end_col: col,
                                        }));
                                        prev_screen_left = None;
                                        prev_screen_right = None;
                                        pty_needs_render = true;
                                    }
                                }
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    if let Some((pane, ref mut sel)) = selection {
                                        let (row, col) = clamp_to_pane(pane, &app, mouse.column, mouse.row, term_cols, term_rows);
                                        sel.end_row = row;
                                        sel.end_col = col;
                                        prev_screen_left = None;
                                        prev_screen_right = None;
                                        pty_needs_render = true;
                                    }
                                }
                                MouseEventKind::Up(MouseButton::Left) => {
                                    if let Some((pane, ref sel)) = selection {
                                        if sel.is_non_empty() {
                                            let screen = match pane {
                                                Pane::Left => app.pty_session.as_ref().map(|p| p.screen()),
                                                Pane::Right => app.pty_right.as_ref().map(|p| p.screen()),
                                            };
                                            if let Some(screen) = screen {
                                                let (inner_w, inner_h) = pane_inner_size(pane, &app, term_cols, term_rows);
                                                let text = ui::extract_selection_text(screen, sel, inner_w, inner_h);
                                                if !text.is_empty() {
                                                    copy_to_clipboard(&text);
                                                }
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::ScrollUp => {
                                    let offset = match app.active_pane {
                                        Pane::Left => &mut scroll_offset_left,
                                        Pane::Right => &mut scroll_offset_right,
                                    };
                                    *offset += 3;
                                    pty_needs_render = true;
                                }
                                MouseEventKind::ScrollDown => {
                                    let offset = match app.active_pane {
                                        Pane::Left => &mut scroll_offset_left,
                                        Pane::Right => &mut scroll_offset_right,
                                    };
                                    *offset = offset.saturating_sub(3);
                                    pty_needs_render = true;
                                }
                                _ => {}
                            }
                        }
                    } else {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => app.move_up(),
                            MouseEventKind::ScrollDown => app.move_down(),
                            _ => {}
                        }
                    }
                }
                Event::Resize(cols, rows) => {
                    term_cols = cols;
                    term_rows = rows;
                    if app.mode == Mode::Attached {
                        app.resize_pty(rows, cols);
                        pty_needs_render = true;
                        ui_needs_draw = true;
                        prev_screen_left = None;
                        prev_screen_right = None;
                        scroll_offset_left = 0;
                        scroll_offset_right = 0;
                    }
                }
                _ => {}
            }
        }
    }

    // Teardown terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;

    // Write action file if configured (only GoHome uses it now)
    if let Some(ref path) = app.action_file {
        match &exit_action {
            Action::GoHome => {
                fs::write(path, "")?;
            }
            Action::None => {
                let _ = fs::remove_file(path);
            }
        }
    }

    Ok(())
}

/// Determine which pane a mouse click falls in, returning pane-local (row, col).
fn hit_test_pane(app: &App, mx: u16, my: u16, cols: u16, rows: u16) -> Option<(Pane, u16, u16)> {
    if app.pty_right.is_some() {
        let (left_x, left_w, right_x, right_w, inner_y, inner_h) =
            if app.sidebar_mode {
                ui::sidebar_two_pane_geometry(app, cols, rows)
            } else {
                ui::two_pane_geometry(cols, rows)
            };
        if my >= inner_y && my < inner_y + inner_h {
            if mx >= left_x && mx < left_x + left_w {
                return Some((Pane::Left, my - inner_y, mx - left_x));
            }
            if mx >= right_x && mx < right_x + right_w {
                return Some((Pane::Right, my - inner_y, mx - right_x));
            }
        }
    } else {
        let (inner_x, inner_y, inner_w, inner_h) = if app.sidebar_mode {
            ui::sidebar_geometry(app, cols, rows)
        } else {
            (1u16, 1u16, cols.saturating_sub(2), rows.saturating_sub(2))
        };
        if my >= inner_y && my < inner_y + inner_h && mx >= inner_x && mx < inner_x + inner_w {
            return Some((Pane::Left, my - inner_y, mx - inner_x));
        }
    }
    None
}

/// Clamp mouse coordinates to pane boundaries, returning pane-local (row, col).
fn clamp_to_pane(pane: Pane, app: &App, mx: u16, my: u16, cols: u16, rows: u16) -> (u16, u16) {
    if app.pty_right.is_some() {
        let (left_x, left_w, right_x, right_w, inner_y, inner_h) =
            if app.sidebar_mode {
                ui::sidebar_two_pane_geometry(app, cols, rows)
            } else {
                ui::two_pane_geometry(cols, rows)
            };
        let (px, pw) = match pane {
            Pane::Left => (left_x, left_w),
            Pane::Right => (right_x, right_w),
        };
        let row = my.max(inner_y).min(inner_y + inner_h - 1) - inner_y;
        let col = mx.max(px).min(px + pw - 1) - px;
        (row, col)
    } else {
        let (inner_x, inner_y, inner_w, inner_h) = if app.sidebar_mode {
            ui::sidebar_geometry(app, cols, rows)
        } else {
            (1u16, 1u16, cols.saturating_sub(2), rows.saturating_sub(2))
        };
        let row = my.max(inner_y).min(inner_y + inner_h - 1) - inner_y;
        let col = mx.max(inner_x).min(inner_x + inner_w - 1) - inner_x;
        (row, col)
    }
}

/// Get inner dimensions (width, height) for a specific pane.
fn pane_inner_size(pane: Pane, app: &App, cols: u16, rows: u16) -> (u16, u16) {
    if app.pty_right.is_some() {
        let (_, left_w, _, right_w, _, inner_h) = if app.sidebar_mode {
            ui::sidebar_two_pane_geometry(app, cols, rows)
        } else {
            ui::two_pane_geometry(cols, rows)
        };
        let w = match pane {
            Pane::Left => left_w,
            Pane::Right => right_w,
        };
        (w, inner_h)
    } else {
        let (_, _, inner_w, inner_h) = if app.sidebar_mode {
            ui::sidebar_geometry(app, cols, rows)
        } else {
            (1u16, 1u16, cols.saturating_sub(2), rows.saturating_sub(2))
        };
        (inner_w, inner_h)
    }
}

/// Copy text to system clipboard using pbcopy.
fn copy_to_clipboard(text: &str) {
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}
