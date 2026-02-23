pub fn init_script(shell: &str) -> Result<String, String> {
    match shell {
        "zsh" => Ok(zsh_script()),
        "bash" => Ok(bash_script()),
        _ => Err(format!("Unsupported shell: {shell}. Use 'zsh' or 'bash'.")),
    }
}

fn zsh_script() -> String {
    r#"# scrn shell integration (zsh)
# Add to .zshrc: eval "$(scrn init zsh)"

scrn() {
    case "${1:-}" in
        init|--version|-v|--help|-h)
            command scrn "$@"
            return
            ;;
    esac

    # Disable Ctrl+S flow control so it can be used as detach key in screen
    stty -ixon 2>/dev/null
    command scrn "$@"
    stty ixon 2>/dev/null
}
"#
    .to_string()
}

fn bash_script() -> String {
    r#"# scrn shell integration (bash)
# Add to .bashrc: eval "$(scrn init bash)"

scrn() {
    case "${1:-}" in
        init|--version|-v|--help|-h)
            command scrn "$@"
            return
            ;;
    esac

    # Disable Ctrl+S flow control so it can be used as detach key in screen
    stty -ixon 2>/dev/null
    command scrn "$@"
    stty ixon 2>/dev/null
}
"#
    .to_string()
}
