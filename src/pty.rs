use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct PtySession {
    master: OwnedFd,
    child: Child,
    parser: vt100::Parser,
}

impl PtySession {
    pub fn spawn(program: &str, args: &[&str], rows: u16, cols: u16) -> io::Result<Self> {
        Self::spawn_inner(program, args, rows, cols, None)
    }

    pub fn spawn_in_dir(
        program: &str,
        args: &[&str],
        rows: u16,
        cols: u16,
        dir: &std::path::Path,
    ) -> io::Result<Self> {
        Self::spawn_inner(program, args, rows, cols, Some(dir))
    }

    fn spawn_inner(
        program: &str,
        args: &[&str],
        rows: u16,
        cols: u16,
        dir: Option<&std::path::Path>,
    ) -> io::Result<Self> {
        let (master, slave) = open_pty()?;
        set_pty_size(master.as_raw_fd(), rows, cols);
        set_nonblocking(master.as_raw_fd())?;

        let slave_raw = slave.into_raw_fd();
        let dup1 = dup_fd(slave_raw)?;
        let dup2 = dup_fd(slave_raw)?;

        let mut cmd = Command::new(program);
        cmd.args(args)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env_remove("STY")
            .env_remove("WINDOW");
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
        let child = unsafe {
            cmd.stdin(Stdio::from_raw_fd(slave_raw))
                .stdout(Stdio::from_raw_fd(dup1))
                .stderr(Stdio::from_raw_fd(dup2))
                .pre_exec(|| {
                    libc::setsid();
                    Ok(())
                })
                .spawn()?
        };

        let parser = vt100::Parser::new(rows, cols, 10000);
        Ok(Self {
            master,
            child,
            parser,
        })
    }

    /// Read any available output from the PTY (non-blocking).
    /// Returns `true` if any data was read (screen may have changed).
    pub fn try_read(&mut self) -> bool {
        let mut any = false;
        let mut buf = [0u8; 65536];
        loop {
            let n = unsafe {
                libc::read(
                    self.master.as_raw_fd(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                )
            };
            if n > 0 {
                self.parser.process(&buf[..n as usize]);
                any = true;
            } else {
                break;
            }
        }
        any
    }

    /// Send input bytes to the PTY, handling partial writes and back-pressure.
    pub fn write_all(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let fd = self.master.as_raw_fd();
        let mut offset = 0;
        while offset < data.len() {
            let n = unsafe {
                libc::write(fd, data[offset..].as_ptr().cast(), data[offset..].len())
            };
            if n > 0 {
                offset += n as usize;
            } else {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    // PTY buffer full — wait for it to drain
                    let mut pfd = libc::pollfd {
                        fd,
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    unsafe {
                        libc::poll(&mut pfd, 1, 100);
                    }
                } else {
                    break;
                }
            }
        }
    }

    /// Resize the PTY and internal parser.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        // Drain any pending output at the old size before switching dimensions
        self.try_read();
        self.parser.set_size(rows, cols);
        set_pty_size(self.master.as_raw_fd(), rows, cols);
        // Explicitly signal the child in case TIOCSWINSZ didn't deliver SIGWINCH
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGWINCH);
        }
    }

    /// Whether the terminal is in alternate screen mode.
    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    /// Check if the child process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Get the current terminal screen state.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Set the scrollback offset (0 = live view, >0 = scrolled back).
    pub fn set_scrollback(&mut self, offset: usize) {
        self.parser.set_scrollback(offset);
    }

    /// Current scrollback offset (0 = live view).
    pub fn scrollback_offset(&self) -> usize {
        self.parser.screen().scrollback()
    }

    /// Total number of scrollback rows available.
    #[allow(dead_code)]
    pub fn scrollback_available(&mut self) -> usize {
        let current = self.parser.screen().scrollback();
        self.parser.set_scrollback(usize::MAX);
        let available = self.parser.screen().scrollback();
        self.parser.set_scrollback(current);
        available
    }

    /// Whether the terminal is in application cursor key mode (DECCKM).
    pub fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    /// Raw file descriptor for the PTY master (for multiplexed poll).
    pub fn master_fd(&self) -> i32 {
        self.master.as_raw_fd()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Already exited (e.g. after screen detach) — just reap
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        // SIGTERM for graceful shutdown (lets screen clean up its socket)
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        // Brief wait for clean exit
        let deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Force kill as last resort
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Convert a crossterm KeyEvent to raw bytes for the PTY.
/// When `app_cursor` is true the terminal is in application cursor mode
/// (DECCKM), so arrow keys use `\x1bO` prefix instead of `\x1b[`.
pub fn key_to_bytes(key: &KeyEvent, app_cursor: bool) -> Vec<u8> {
    // Ctrl+key
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = key.code {
            let byte = (c.to_ascii_lowercase() as u8) & 0x1f;
            return vec![byte];
        }
    }

    // Alt+key (ESC prefix)
    if key.modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(c) = key.code {
            let mut bytes = vec![0x1b];
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf);
            bytes.extend_from_slice(&buf[..c.len_utf8()]);
            return bytes;
        }
    }

    // Arrow keys: application mode uses \x1bO prefix, normal uses \x1b[
    let arrow_prefix: &[u8] = if app_cursor { b"\x1bO" } else { b"\x1b[" };

    // Shift+Enter → kitty protocol CSI u encoding so TUI apps can distinguish it
    if key.modifiers.contains(KeyModifiers::SHIFT) && key.code == KeyCode::Enter {
        return b"\x1b[13;2u".to_vec();
    }

    match key.code {
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf);
            buf[..c.len_utf8()].to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => [arrow_prefix, b"A"].concat(),
        KeyCode::Down => [arrow_prefix, b"B"].concat(),
        KeyCode::Right => [arrow_prefix, b"C"].concat(),
        KeyCode::Left => [arrow_prefix, b"D"].concat(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => f_key_bytes(n),
        _ => vec![],
    }
}

fn f_key_bytes(n: u8) -> Vec<u8> {
    match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => vec![],
    }
}

fn open_pty() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let ret = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    unsafe { Ok((OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))) }
}

fn set_pty_size(fd: i32, rows: u16, cols: u16) {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(fd, libc::TIOCSWINSZ, &ws);
    }
}

fn set_nonblocking(fd: i32) -> io::Result<()> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn dup_fd(fd: i32) -> io::Result<i32> {
    let new_fd = unsafe { libc::dup(fd) };
    if new_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(new_fd)
}
