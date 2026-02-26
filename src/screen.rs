use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;

extern crate libc;

#[derive(Clone, Debug)]
pub enum SessionState {
    Attached,
    Detached,
}


#[derive(Clone, Debug)]
pub struct Session {
    pub name: String,
    pub pid_name: String,
    #[allow(dead_code)]
    pub state: SessionState,
    #[allow(dead_code)]
    pub created: Option<u64>,
    #[allow(dead_code)]
    pub idle_secs: Option<u64>,
}

/// Returns the path to scrn's managed screenrc, creating it if needed.
///
/// Sources the user's ~/.screenrc (if it exists), then enables truecolor
/// so that 24-bit color sequences pass through GNU Screen — the same
/// thing tmux/zellij do out of the box.
pub fn ensure_screenrc() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(&home).join(".config").join("scrn");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("screenrc");

    let user_rc = PathBuf::from(&home).join(".screenrc");
    let mut content = String::new();
    if user_rc.exists() {
        content.push_str(&format!("source {}\n", user_rc.display()));
    }
    content.push_str("truecolor on\n");
    content.push_str("defscrollback 10000\n");
    // Pass alternate screen sequences through so full-screen apps
    // render correctly in the native terminal.
    content.push_str("altscreen on\n");
    // Disable flow control so Ctrl+S is not eaten by the terminal driver
    content.push_str("defflow off\n");
    // Ctrl+S detaches from screen, returning to the scrn picker
    content.push_str("bindkey \"^S\" detach\n");

    let _ = fs::write(&path, &content);
    path.to_string_lossy().into_owned()
}

const MIN_SCREEN_MAJOR: u32 = 5;

/// Check that GNU Screen is installed and >= 5.0 (required for truecolor).
pub fn check_version() -> Result<(), String> {
    let output = Command::new("screen")
        .arg("--version")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "GNU Screen is not installed.\n\n\
                 Install it with:\n  \
                 macOS:  brew install screen\n  \
                 Linux:  apt install screen  (or your distro's package manager)"
                    .to_string()
            } else {
                format!("Failed to run screen: {e}")
            }
        })?;

    let text = String::from_utf8_lossy(&output.stdout);
    // Format: "Screen version 5.0.1 ..." or "Screen version 4.00.03 (FAU) ..."
    let version_str = text
        .split_whitespace()
        .nth(2)
        .unwrap_or("unknown");

    let major = version_str
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    if major < MIN_SCREEN_MAJOR {
        Err(format!(
            "GNU Screen {version_str} is too old. scrn requires Screen 5.0+ for truecolor support.\n\n\
             Your screen: {version_str}\n\
             Required:    5.0+\n\n\
             Upgrade with:\n  \
             macOS:  brew install screen\n  \
             Linux:  build from source (https://ftp.gnu.org/gnu/screen/)"
        ))
    } else {
        Ok(())
    }
}

pub fn list_sessions() -> Result<Vec<Session>, String> {
    let output = Command::new("screen")
        .arg("-ls")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "GNU Screen is not installed. Install it with: brew install screen (macOS) or apt install screen (Linux)".to_string()
            } else {
                format!("Failed to run screen: {e}")
            }
        })?;

    let text = String::from_utf8_lossy(&output.stdout);

    // screen -ls returns exit code 1 when sessions exist, 0 when none
    if text.contains("No Sockets found") || text.trim().is_empty() {
        return Ok(Vec::new());
    }

    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut sessions = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.contains('\t') {
            continue;
        }

        // Format: "\t<pid.name>\t(<date>)\t(<State>)"
        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }

        let pid_name = parts[0].trim();
        if !pid_name.contains('.') {
            continue;
        }

        let dot_pos = pid_name.find('.').unwrap();
        let name = &pid_name[dot_pos + 1..];

        let rest = parts[1..].join("\t");

        // Skip dead sessions (process exited but socket lingered)
        if rest.contains("Dead") {
            continue;
        }

        let state = if rest.contains("Attached") {
            SessionState::Attached
        } else {
            SessionState::Detached
        };

        let (created, idle_secs) = std::env::var("HOME")
            .ok()
            .and_then(|home| {
                let socket = PathBuf::from(&home).join(".screen").join(pid_name);
                fs::metadata(&socket).ok().map(|m| {
                    let created = m
                        .created()
                        .ok()
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    let idle = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| now_secs.saturating_sub(d.as_secs()));
                    (created, idle)
                })
            })
            .unwrap_or((None, None));

        sessions.push(Session {
            name: name.to_string(),
            pid_name: pid_name.to_string(),
            state,
            created,
            idle_secs,
        });
    }

    Ok(sessions)
}

pub fn kill_session(pid_name: &str) -> Result<(), String> {
    let pid = pid_name
        .split('.')
        .next()
        .and_then(|s| s.parse::<i32>().ok())
        .ok_or_else(|| format!("Failed to parse PID from '{pid_name}'"))?;

    // SIGTERM first for graceful shutdown
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }

    // Wait up to 500ms for it to die
    for _ in 0..50 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Still alive? SIGKILL
    if unsafe { libc::kill(pid, 0) } == 0 {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // screen -wipe doesn't reliably clean up on screen 5 — remove the socket directly
    let _ = Command::new("screen").arg("-wipe").output();
    if let Ok(home) = std::env::var("HOME") {
        let socket = PathBuf::from(&home).join(".screen").join(pid_name);
        let _ = fs::remove_file(&socket);
    }

    Ok(())
}

pub fn rename_session(pid_name: &str, new_name: &str) -> Result<(), String> {
    let output = Command::new("screen")
        .args(["-S", pid_name, "-X", "sessionname", new_name])
        .output()
        .map_err(|e| format!("Failed to rename session: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to rename session: {}", stderr.trim()))
    }
}

pub fn create_session(name: &str) -> Result<(), String> {
    let rc = ensure_screenrc();
    let output = Command::new("screen")
        .args(["-c", &rc, "-dmS", name])
        .env("COLORTERM", "truecolor")
        .output()
        .map_err(|e| format!("Failed to create session: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to create session: {}", stderr.trim()))
    }
}

const SHELL_NAMES: &[&str] = &[
    "bash", "zsh", "sh", "fish", "dash", "ksh", "tcsh", "csh",
];

/// Strip path prefix and login-shell `-` from a single token (argv[0]).
/// `/bin/zsh` → `zsh`, `-zsh` → `zsh`.
fn argv0_base(token: &str) -> &str {
    let base = token.rsplit('/').next().unwrap_or(token);
    base.strip_prefix('-').unwrap_or(base)
}

/// True when the first word of `args` is a shell or screen itself.
fn is_shell_or_screen(args: &str) -> bool {
    let first = args.split_whitespace().next().unwrap_or(args);
    let base = argv0_base(first);
    base.is_empty() || base == "screen" || SHELL_NAMES.contains(&base)
}

/// Basename any token that looks like an absolute path.
fn normalize_token(token: &str) -> &str {
    if token.starts_with('/') || token.starts_with('~') {
        token.rsplit('/').next().unwrap_or(token)
    } else {
        token
    }
}

/// Replace full paths in argv[0] and any path-like arguments with their basenames.
/// `/usr/local/bin/npm run dev`          → `npm run dev`
/// `node /Users/jens/project/server.js`  → `node server.js`
fn normalize_args(args: &str) -> String {
    let mut tokens = args.split_whitespace();
    let Some(argv0) = tokens.next() else { return String::new() };
    let mut parts = vec![argv0_base(argv0).to_string()];
    for token in tokens {
        parts.push(normalize_token(token).to_string());
    }
    parts.join(" ")
}

/// Parsed output of a single `ps -axo pid=,ppid=,args=` call.
/// Can be built independently of knowing which session PIDs to query.
pub struct ProcessMap {
    args_map: HashMap<u32, String>,
    children: HashMap<u32, Vec<u32>>,
}

impl Default for ProcessMap {
    fn default() -> Self {
        Self {
            args_map: HashMap::new(),
            children: HashMap::new(),
        }
    }
}

/// Run `ps -axo` once and return the parsed process tree.
/// This is the slow part (subprocess); split from the BFS so it can run in parallel.
pub fn build_process_map() -> ProcessMap {
    let output = match Command::new("ps").args(["-axo", "pid=,ppid=,args="]).output() {
        Ok(o) => o,
        Err(_) => return ProcessMap::default(),
    };
    let text = String::from_utf8_lossy(&output.stdout);

    let mut args_map: HashMap<u32, String> = HashMap::new();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();

    for line in text.lines() {
        let trimmed = line.trim();
        let Some((pid_str, rest)) = trimmed.split_once(|c: char| c.is_ascii_whitespace()) else { continue };
        let rest = rest.trim_start();
        let Some((ppid_str, args_str)) = rest.split_once(|c: char| c.is_ascii_whitespace()) else { continue };
        let Some(pid) = pid_str.parse::<u32>().ok() else { continue };
        let Some(ppid) = ppid_str.parse::<u32>().ok() else { continue };
        let args = args_str.trim_start().to_string();
        args_map.insert(pid, args);
        children.entry(ppid).or_default().push(pid);
    }

    ProcessMap { args_map, children }
}

/// BFS through a pre-built process map to find the foreground process for each session.
/// Pure computation — no subprocess.
pub fn foreground_from_map(map: &ProcessMap, session_pids: &[u32]) -> HashMap<u32, String> {
    if session_pids.is_empty() {
        return HashMap::new();
    }

    let mut result: HashMap<u32, String> = HashMap::new();
    'outer: for &screen_pid in session_pids {
        let mut frontier = vec![screen_pid];
        let mut visited = std::collections::HashSet::new();
        visited.insert(screen_pid);

        for _ in 0..4 {
            let mut next = Vec::new();
            for pid in frontier {
                let Some(kids) = map.children.get(&pid) else { continue };
                for &kid in kids {
                    if !visited.insert(kid) {
                        continue;
                    }
                    let args = map.args_map.get(&kid).map(|s| s.as_str()).unwrap_or("");
                    if is_shell_or_screen(args) {
                        next.push(kid);
                    } else {
                        result.insert(screen_pid, normalize_args(args));
                        continue 'outer;
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
    }
    result
}


fn get_process_cwd(pid: u32) -> Option<PathBuf> {
    // Linux: /proc/<pid>/cwd symlink
    let proc_cwd = PathBuf::from(format!("/proc/{pid}/cwd"));
    if let Ok(path) = fs::read_link(&proc_cwd) {
        return Some(path);
    }

    // macOS: lsof -p <pid> -a -d cwd -Fn
    let output = Command::new("lsof")
        .args(["-p", &pid.to_string(), "-a", "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(path) = line.strip_prefix('n') {
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

/// Returns the current working directory of the shell inside the given screen session.
pub fn get_session_cwd(pid_name: &str) -> Option<PathBuf> {
    let screen_pid: u32 = pid_name.split('.').next()?.parse().ok()?;

    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,args="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut args_map: HashMap<u32, String> = HashMap::new();

    for line in text.lines() {
        let trimmed = line.trim();
        let Some((pid_str, rest)) = trimmed.split_once(|c: char| c.is_ascii_whitespace()) else { continue };
        let rest = rest.trim_start();
        let Some((ppid_str, args_str)) = rest.split_once(|c: char| c.is_ascii_whitespace()) else { continue };
        let Some(pid) = pid_str.parse::<u32>().ok() else { continue };
        let Some(ppid) = ppid_str.parse::<u32>().ok() else { continue };
        args_map.insert(pid, args_str.trim_start().to_string());
        children.entry(ppid).or_default().push(pid);
    }

    // BFS through shell/screen children to find the innermost shell
    let mut frontier = vec![screen_pid];
    let mut visited = std::collections::HashSet::new();
    visited.insert(screen_pid);

    for _ in 0..4 {
        let mut next = Vec::new();
        for &pid in &frontier {
            let Some(kids) = children.get(&pid) else { continue };
            for &kid in kids {
                if !visited.insert(kid) {
                    continue;
                }
                let args = args_map.get(&kid).map(|s| s.as_str()).unwrap_or("");
                if is_shell_or_screen(args) {
                    next.push(kid);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    for pid in frontier {
        if let Some(cwd) = get_process_cwd(pid) {
            return Some(cwd);
        }
    }
    None
}

pub fn create_session_in_dir(name: &str, dir: &std::path::Path) -> Result<(), String> {
    let rc = ensure_screenrc();
    let output = Command::new("screen")
        .args(["-c", &rc, "-dmS", name])
        .current_dir(dir)
        .env("COLORTERM", "truecolor")
        .output()
        .map_err(|e| format!("Failed to create session: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to create session: {}", stderr.trim()))
    }
}


