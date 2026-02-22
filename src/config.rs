use std::fs;
use std::path::PathBuf;

pub struct Config {
    pub workspace: Option<PathBuf>,
    pub sidebar: bool,
    /// Default split ratio (20-80) from config.toml; used when no persisted override exists.
    pub split_ratio: Option<u32>,
}

impl Config {
    pub fn load(cli_workspace: Option<&str>) -> Self {
        // Always read config file first for all settings
        let mut workspace = None;
        let mut sidebar = false;
        let mut split_ratio = None;

        if let Some(contents) = read_config_file() {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim().trim_matches('"');
                    match key {
                        "workspace" if !value.is_empty() => {
                            workspace = Some(expand_tilde(value));
                        }
                        "sidebar" => {
                            sidebar = value == "true";
                        }
                        // Default left/right split ratio (20-80) for two-pane mode.
                        "split_ratio" => {
                            if let Ok(pct) = value.parse::<u32>() {
                                if (20..=80).contains(&pct) {
                                    split_ratio = Some(pct);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // CLI arg takes precedence for workspace
        if let Some(ws) = cli_workspace {
            workspace = Some(expand_tilde(ws));
        }

        Self { workspace, sidebar, split_ratio }
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
