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
    local action_file
    action_file=$(mktemp "${TMPDIR:-/tmp}/scrn-action.XXXXXX")

    command scrn --action-file "$action_file" "$@"

    if [ -f "$action_file" ]; then
        local action
        action=$(cat "$action_file")
        rm -f "$action_file"

        if [ -n "$action" ]; then
            if [ -n "$STY" ]; then
                # Inside a screen session: write to pending file, then detach
                local pending_file="${TMPDIR:-/tmp}/scrn-pending-$$-$(date +%s)"
                echo "$action" > "$pending_file"
                # Store pending file path for the precmd hook on the outer shell
                echo "$pending_file" > "${TMPDIR:-/tmp}/scrn-pending-path"
                screen -X detach
            else
                eval "$action"
            fi
        fi
    else
        rm -f "$action_file"
    fi
}

_scrn_precmd() {
    local pending_path_file="${TMPDIR:-/tmp}/scrn-pending-path"
    if [ -f "$pending_path_file" ]; then
        local pending_file
        pending_file=$(cat "$pending_path_file")
        rm -f "$pending_path_file"
        if [ -f "$pending_file" ]; then
            local action
            action=$(cat "$pending_file")
            rm -f "$pending_file"
            if [ -n "$action" ]; then
                eval "$action"
            fi
        fi
    fi
}

if [[ -z "${precmd_functions[(r)_scrn_precmd]}" ]]; then
    precmd_functions+=(_scrn_precmd)
fi
"#
    .to_string()
}

fn bash_script() -> String {
    r#"# scrn shell integration (bash)
# Add to .bashrc: eval "$(scrn init bash)"

scrn() {
    local action_file
    action_file=$(mktemp "${TMPDIR:-/tmp}/scrn-action.XXXXXX")

    command scrn --action-file "$action_file" "$@"

    if [ -f "$action_file" ]; then
        local action
        action=$(cat "$action_file")
        rm -f "$action_file"

        if [ -n "$action" ]; then
            if [ -n "$STY" ]; then
                local pending_file="${TMPDIR:-/tmp}/scrn-pending-$$-$(date +%s)"
                echo "$action" > "$pending_file"
                echo "$pending_file" > "${TMPDIR:-/tmp}/scrn-pending-path"
                screen -X detach
            else
                eval "$action"
            fi
        fi
    else
        rm -f "$action_file"
    fi
}

_scrn_precmd() {
    local pending_path_file="${TMPDIR:-/tmp}/scrn-pending-path"
    if [ -f "$pending_path_file" ]; then
        local pending_file
        pending_file=$(cat "$pending_path_file")
        rm -f "$pending_path_file"
        if [ -f "$pending_file" ]; then
            local action
            action=$(cat "$pending_file")
            rm -f "$pending_file"
            if [ -n "$action" ]; then
                eval "$action"
            fi
        fi
    fi
}

PROMPT_COMMAND="_scrn_precmd;${PROMPT_COMMAND}"
"#
    .to_string()
}
