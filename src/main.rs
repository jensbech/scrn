mod app;
mod config;
mod logging;
mod screen;
mod shell;
mod ui;
mod workspace;

use std::io;
use std::process::Command;
use std::sync::mpsc::Receiver;
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

use app::{Action, App, Mode};

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
    let mut cli_workspace = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
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

    // Disable flow control so Ctrl+S reaches screen as the detach key
    disable_flow_control();

    let mut app = App::new(cfg.workspace);
    app.refresh_sessions();
    app.restore_sessions();

    // Set up terminal once for the whole session lifetime — no flash between cycles.
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

    // Main loop: show picker → spawn screen → wait for detach → repeat
    let mut pending_refresh: Option<Receiver<app::RefreshData>> = None;
    loop {
        let action = run_picker(&mut app, &mut terminal, pending_refresh.take())?;

        match action {
            Action::Attach(ref pid_name) => {
                yield_terminal(&mut terminal)?;

                let pn1 = pid_name.clone();
                let pn2 = pid_name.clone();
                let t1 = std::thread::spawn(move || {
                    Command::new("screen")
                        .args(["-S", &pn1, "-X", "bindkey", "^S", "detach"])
                        .status()
                });
                let t2 = std::thread::spawn(move || {
                    Command::new("screen")
                        .args(["-S", &pn2, "-X", "defflow", "off"])
                        .status()
                });
                t1.join().ok();
                t2.join().ok();

                let session_name = pid_name.split('.').nth(1).unwrap_or(pid_name);
                if let Some(cmd) = app.constant_command(session_name) {
                    let stuff = format!("{}\n", cmd);
                    let _ = Command::new("screen")
                        .args(["-S", pid_name, "-X", "stuff", &stuff])
                        .status();
                }

                let rc = screen::ensure_screenrc();
                let _ = Command::new("screen")
                    .args(["-c", &rc, "-d", "-r", pid_name])
                    .status();

                reclaim_terminal(&mut terminal)?;
                app.action = Action::None;
                pending_refresh = Some(app::spawn_refresh(
                    app.workspace_dir.clone(),
                    app.dir_order.clone(),
                ));
            }
            Action::Create(ref name, ref maybe_dir) => {
                yield_terminal(&mut terminal)?;
                let rc = screen::ensure_screenrc();

                let has_command = app.constant_command(name).is_some();

                if has_command {
                    let mut cmd = Command::new("screen");
                    cmd.args(["-c", &rc, "-dmS", name]);
                    if let Some(ref dir) = maybe_dir {
                        cmd.current_dir(dir);
                    }
                    let _ = cmd.status();

                    if let Some(c) = app.constant_command(name) {
                        let stuff = format!("{}\n", c);
                        let _ = Command::new("screen")
                            .args(["-S", name, "-X", "stuff", &stuff])
                            .status();
                    }

                    let _ = Command::new("screen")
                        .args(["-c", &rc, "-r", name])
                        .status();
                } else {
                    let mut cmd = Command::new("screen");
                    cmd.args(["-c", &rc, "-S", name]);
                    if let Some(ref dir) = maybe_dir {
                        cmd.current_dir(dir);
                    }
                    let _ = cmd.status();
                }

                reclaim_terminal(&mut terminal)?;
                app.action = Action::None;
                pending_refresh = Some(app::spawn_refresh(
                    app.workspace_dir.clone(),
                    app.dir_order.clone(),
                ));
            }
            Action::Quit | Action::None => break,
        }
    }

    app.kill_all_throwaway();

    // Final teardown
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;

    Ok(())
}

/// Temporarily restore the terminal to its normal state so screen can take it over.
fn yield_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    Ok(())
}

/// Reclaim the terminal after screen exits — enter alternate screen and redraw.
fn reclaim_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;
    terminal.clear()?;
    Ok(())
}

fn hit_test_row(app: &App, row: u16) -> Option<usize> {
    if row < app.table_data_y || row >= app.table_data_end_y {
        return None;
    }
    let visual_idx = (row - app.table_data_y) as usize + app.table_scroll_offset;
    app.selectable_indices.iter().position(|&si| si == visual_idx)
}

/// Show the TUI session picker and return the user's chosen action.
/// If `refresh_rx` is provided, the UI starts immediately with current (possibly stale)
/// data and applies the fresh data as soon as the background thread delivers it.
fn run_picker(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    refresh_rx: Option<Receiver<app::RefreshData>>,
) -> Result<Action, Box<dyn std::error::Error>> {
    let mut refresh_rx = refresh_rx;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // Apply background refresh data as soon as it arrives
        if let Some(ref rx) = refresh_rx {
            if let Ok(data) = rx.try_recv() {
                app.apply_refresh_data(data);
                refresh_rx = None;
            }
        }

        // Auto-clear stale status messages
        if !app.status_msg.is_empty()
            && app.status_set_at.elapsed() > Duration::from_secs(5)
        {
            app.status_msg.clear();
        }

        // Check if an action was triggered
        if !matches!(app.action, Action::None) {
            break;
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.mode {
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
                            app.select_for_attach();
                        }
                        KeyCode::Char('c') => app.start_create(),
                        KeyCode::Char('n') => app.start_rename(),
                        KeyCode::Char('x') => app.start_kill(),
                        KeyCode::Char('X') => app.start_kill_all(),
                        KeyCode::Char('/') => app.start_search(),
                        KeyCode::Char('p') => app.start_pin_confirm(),
                        KeyCode::Char('C') => app.start_constant_confirm(),
                        KeyCode::Tab => {
                            if !app.search_input.is_empty() {
                                app.toggle_search_filter();
                            }
                        }
                        KeyCode::Char('r') => app.refresh_sessions(),
                        KeyCode::Char('t') => app.create_throwaway(),
                        KeyCode::Char('d') => app.duplicate_session(),
                        KeyCode::Char('s') => app.start_note_edit(),
                        KeyCode::Char('O') => app.start_ordering(),
                        KeyCode::Char('R') => app.start_constant_ordering(),
                        KeyCode::Char(ch @ '1'..='9') => app.select_constant(ch as usize - '0' as usize),
                        _ => {}
                    },
                    Mode::Searching => match key.code {
                        KeyCode::Esc => app.confirm_search(),
                        KeyCode::Enter => {
                            app.confirm_search();
                            app.select_for_attach();
                        }
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
                    Mode::EditingNote => match key.code {
                        KeyCode::Enter => app.confirm_note(),
                        KeyCode::Esc => app.cancel_note(),
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
                    Mode::ConfirmPin => match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => app.confirm_pin(),
                        KeyCode::Char('n') | KeyCode::Esc => app.cancel_pin(),
                        _ => {}
                    },
                    Mode::ConfirmConstant => match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => app.confirm_constant(),
                        KeyCode::Char('n') | KeyCode::Esc => app.cancel_constant(),
                        _ => {}
                    },
                    Mode::ConfirmKill => match key.code {
                        KeyCode::Char('y') => app.confirm_kill(),
                        KeyCode::Esc => app.cancel_kill(),
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
                            app.action = Action::Quit;
                        }
                        KeyCode::Char('n') | KeyCode::Esc => {
                            app.mode = Mode::Normal;
                        }
                        _ => {}
                    },
                    Mode::Ordering => match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            if app.ordering_selected + 1 < app.ordering_items.len() {
                                app.ordering_selected += 1;
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if app.ordering_selected > 0 {
                                app.ordering_selected -= 1;
                            }
                        }
                        KeyCode::Char('J') => {
                            if app.ordering_selected + 1 < app.ordering_items.len() {
                                app.ordering_items.swap(app.ordering_selected, app.ordering_selected + 1);
                                app.ordering_selected += 1;
                            }
                        }
                        KeyCode::Char('K') => {
                            if app.ordering_selected > 0 {
                                app.ordering_items.swap(app.ordering_selected, app.ordering_selected - 1);
                                app.ordering_selected -= 1;
                            }
                        }
                        KeyCode::Enter => app.confirm_ordering(),
                        KeyCode::Esc => app.cancel_ordering(),
                        _ => {}
                    },
                    Mode::ConstantOrdering => match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            if app.ordering_selected + 1 < app.ordering_items.len() {
                                app.ordering_selected += 1;
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if app.ordering_selected > 0 {
                                app.ordering_selected -= 1;
                            }
                        }
                        KeyCode::Char('J') => {
                            if app.ordering_selected + 1 < app.ordering_items.len() {
                                app.ordering_items.swap(app.ordering_selected, app.ordering_selected + 1);
                                app.ordering_selected += 1;
                            }
                        }
                        KeyCode::Char('K') => {
                            if app.ordering_selected > 0 {
                                app.ordering_items.swap(app.ordering_selected, app.ordering_selected - 1);
                                app.ordering_selected -= 1;
                            }
                        }
                        KeyCode::Enter => app.confirm_constant_ordering(),
                        KeyCode::Esc => app.cancel_constant_ordering(),
                        _ => {}
                    },
                },
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => app.move_up(),
                        MouseEventKind::ScrollDown => app.move_down(),
                        MouseEventKind::Down(MouseButton::Left) => {
                            if matches!(app.mode, Mode::Normal | Mode::Searching) {
                                if let Some(idx) = hit_test_row(app, mouse.row) {
                                    let now = Instant::now();
                                    let is_double = app.last_click
                                        .as_ref()
                                        .map(|(t, prev)| t.elapsed() < Duration::from_millis(400) && *prev == idx)
                                        .unwrap_or(false);
                                    app.selected = idx;
                                    if is_double {
                                        app.last_click = None;
                                        if app.mode == Mode::Searching {
                                            app.confirm_search();
                                        }
                                        app.select_for_attach();
                                    } else {
                                        app.last_click = Some((now, idx));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {} // ratatui handles
                _ => {}
            }
        }
    }

    // Move the action out so we can return it
    let action = std::mem::replace(&mut app.action, Action::None);
    Ok(action)
}

/// Disable terminal flow control (Ctrl+S / Ctrl+Q) so Ctrl+S reaches screen.
fn disable_flow_control() {
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut termios) == 0 {
            termios.c_iflag &= !libc::IXON;
            libc::tcsetattr(0, libc::TCSANOW, &termios);
        }
    }
}
