mod app;
mod config;
mod logging;
mod screen;
mod shell;
mod ui;
mod workspace;

use std::io;
use std::process::Command;
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

    // Main loop: show picker → spawn screen → wait for detach → repeat
    loop {
        let action = run_picker(&mut app)?;

        match action {
            Action::Attach(ref pid_name) => {
                // Inject keybinding + flow control into existing sessions
                // (screenrc is only read at session creation time)
                let _ = Command::new("screen")
                    .args(["-S", pid_name, "-X", "bindkey", "^S", "detach"])
                    .status();
                let _ = Command::new("screen")
                    .args(["-S", pid_name, "-X", "defflow", "off"])
                    .status();
                let rc = screen::ensure_screenrc();
                let _ = Command::new("screen")
                    .args(["-c", &rc, "-d", "-r", pid_name])
                    .status();
                // Screen exited (user detached) — refresh and loop back to picker
                app.action = Action::None;
                app.refresh_sessions();
            }
            Action::Create(ref name, Some(ref dir)) => {
                let rc = screen::ensure_screenrc();
                let _ = Command::new("screen")
                    .args(["-c", &rc, "-S", name])
                    .current_dir(dir)
                    .status();
                app.action = Action::None;
                app.refresh_sessions();
            }
            Action::Create(ref name, None) => {
                let rc = screen::ensure_screenrc();
                let _ = Command::new("screen")
                    .args(["-c", &rc, "-S", name])
                    .status();
                app.action = Action::None;
                app.refresh_sessions();
            }
            Action::Quit | Action::None => break,
        }
    }

    app.kill_all_throwaway();

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
fn run_picker(app: &mut App) -> Result<Action, Box<dyn std::error::Error>> {
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

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

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

    // Teardown terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;

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
