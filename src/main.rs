mod app;
mod config;
mod pty;
mod screen;
mod shell;
mod ui;
mod workspace;

use std::fs;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{Action, App, Mode, Pane};

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
            _ => {}
        }
        i += 1;
    }

    let cfg = config::Config::load(cli_workspace.as_deref());

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    terminal.clear()?;

    let mut app = App::new(action_file, cfg.workspace);
    app.refresh_sessions();

    let exit_action;
    let mut last_esc: Option<Instant> = None;

    loop {
        // Read PTY output before drawing (so display is up to date)
        if app.mode == Mode::Attached {
            let should_detach = if let Some(ref mut pty) = app.pty_session {
                pty.try_read();
                !pty.is_running()
            } else {
                true
            };
            // Also read right pane
            if let Some(ref mut pty_right) = app.pty_right {
                pty_right.try_read();
                if !pty_right.is_running() {
                    // Right pane died — detach both
                    app.detach_pty();
                    continue;
                }
            }
            if should_detach {
                app.detach_pty();
            }
        }

        terminal.draw(|f| ui::draw(f, &app))?;

        // Render PTY content directly to the terminal, bypassing ratatui/crossterm.
        if app.mode == Mode::Attached {
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

            if app.pty_right.is_some() {
                // Two-pane mode
                let (left_x, left_w, right_x, right_w, inner_y, inner_h) =
                    ui::two_pane_geometry(cols, rows);

                if let Some(ref pty) = app.pty_session {
                    let is_active = app.active_pane == Pane::Left;
                    ui::render_pty_direct(
                        terminal.backend_mut(),
                        pty.screen(),
                        left_x,
                        inner_y,
                        left_w,
                        inner_h,
                        is_active,
                    )?;
                }
                if let Some(ref pty) = app.pty_right {
                    let is_active = app.active_pane == Pane::Right;
                    ui::render_pty_direct(
                        terminal.backend_mut(),
                        pty.screen(),
                        right_x,
                        inner_y,
                        right_w,
                        inner_h,
                        is_active,
                    )?;
                }
            } else {
                // Single pane
                if let Some(ref pty) = app.pty_session {
                    let inner_x = 1u16;
                    let inner_y = 1u16;
                    let inner_w = cols.saturating_sub(2);
                    let inner_h = rows.saturating_sub(3);
                    ui::render_pty_direct(
                        terminal.backend_mut(),
                        pty.screen(),
                        inner_x,
                        inner_y,
                        inner_w,
                        inner_h,
                        true,
                    )?;
                }
            }
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

        // Poll faster when attached (smooth terminal), slower for list view
        let poll_duration = if app.mode == Mode::Attached {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(100)
        };

        if event::poll(poll_duration)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.mode {
                    Mode::Attached => {
                        if key.code == KeyCode::Esc {
                            if last_esc.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                                // Double Esc — detach
                                last_esc = None;
                                app.detach_pty();
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
                        } else if key.code == KeyCode::F(6) {
                            // F6 — swap active pane
                            last_esc = None;
                            app.swap_pane();
                        } else {
                            last_esc = None;
                            // Forward everything else to the active pane's PTY
                            let bytes = pty::key_to_bytes(&key);
                            let active_pty = match app.active_pane {
                                Pane::Left => app.pty_session.as_ref(),
                                Pane::Right => app.pty_right.as_ref(),
                            };
                            if let Some(pty) = active_pty {
                                pty.write_all(&bytes);
                            }
                        }
                    }
                    Mode::Normal => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            exit_action = Action::None;
                            break;
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                        KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                        KeyCode::Enter => {
                            let (cols, rows) =
                                crossterm::terminal::size().unwrap_or((80, 24));
                            app.attach_selected(rows, cols);
                        }
                        KeyCode::Char('c') => app.start_create(),
                        KeyCode::Char('n') => app.start_rename(),
                        KeyCode::Char('x') => app.start_kill(),
                        KeyCode::Char('d') => app.go_home(),
                        KeyCode::Char('/') => app.start_search(),
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
                },
                Event::Mouse(mouse) if app.mode != Mode::Attached => match mouse.kind {
                    MouseEventKind::ScrollUp => app.move_up(),
                    MouseEventKind::ScrollDown => app.move_down(),
                    _ => {}
                },
                Event::Resize(cols, rows) => {
                    if app.mode == Mode::Attached {
                        app.resize_pty(rows, cols);
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
