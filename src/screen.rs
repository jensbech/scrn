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
    pub date: String,
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

        let state = if rest.contains("Attached") {
            SessionState::Attached
        } else {
            SessionState::Detached
        };

        // Extract date from parentheses
        let date = rest
            .find('(')
            .and_then(|start| {
                rest[start + 1..].find(')').map(|end| {
                    let d = &rest[start + 1..start + 1 + end];
                    // Skip if it's just "Attached" or "Detached"
                    if d == "Attached" || d == "Detached" {
                        String::new()
                    } else {
                        d.to_string()
                    }
                })
            })
            .unwrap_or_default();

        sessions.push(Session {
            name: name.to_string(),
            pid_name: pid_name.to_string(),
            state,
            date,
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
    let output = Command::new("screen")
        .args(["-dmS", name])
        .output()
        .map_err(|e| format!("Failed to create session: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to create session: {}", stderr.trim()))
    }
}
