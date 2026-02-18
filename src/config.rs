use std::fs;
use std::path::PathBuf;

pub struct Config {
    pub workspace: Option<PathBuf>,
}

impl Config {
    pub fn load(cli_workspace: Option<&str>) -> Self {
        // CLI arg takes precedence
        if let Some(ws) = cli_workspace {
            return Self {
                workspace: Some(expand_tilde(ws)),
            };
        }

        // Try config file
        let workspace = read_config_file().and_then(|contents| {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim().trim_matches('"');
                    if key == "workspace" && !value.is_empty() {
                        return Some(expand_tilde(value));
                    }
                }
            }
            None
        });

        Self { workspace }
    }
}

fn read_config_file() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home)
        .join(".config")
        .join("scrn")
        .join("config.toml");
    fs::read_to_string(path).ok()
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    } else if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}
