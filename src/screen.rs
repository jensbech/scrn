use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug)]
pub enum SessionState {
    Attached,
    Detached,
}

impl SessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionState::Attached => "Attached",
            SessionState::Detached => "Detached",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub name: String,
    pub pid_name: String,
    pub state: SessionState,
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

        sessions.push(Session {
            name: name.to_string(),
            pid_name: pid_name.to_string(),
            state,
        });
    }

    Ok(sessions)
}

pub fn kill_session(pid_name: &str) -> Result<(), String> {
    let output = Command::new("screen")
        .args(["-X", "-S", pid_name, "quit"])
        .output()
        .map_err(|e| format!("Failed to kill session: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to kill session: {}", stderr.trim()))
    }
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


