<p align="center">
  <img src="logo.svg" alt="scrn" width="300" />
</p>

<h1 align="center">scrn</h1>

<p align="center">A terminal UI for managing GNU Screen sessions.</p>

## Requirements

GNU Screen **5.0+** is required for truecolor support. On macOS, run `brew install screen` to get it. scrn checks at startup and will tell you if your version is too old.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/jensbech/scrn/main/install | bash
```

Binaries available for macOS (ARM64/x86_64) and Linux (x86_64/ARM64).

## Setup

```bash
scrn   # launch
```

## Features

- Browse and manage Screen sessions in an interactive table
- Create, rename, and kill sessions
- Seamless session-to-session jumping without nesting
- Search and filter sessions with fuzzy matching
- Embedded PTY display when attached
- Shell integration for zsh and bash
- Workspace mode with tree view and two-pane split

## Workspace mode

Point scrn at a directory of git repos and it displays them as a tree. Selecting a repo opens a two-pane split: left pane for the repo's Screen session, right pane for a companion session (e.g. editor + terminal side by side). Sessions are created automatically on first open and reattached on subsequent visits.

Configure via `~/.config/scrn/config.toml`:

```toml
workspace = "~/projects"
```

Or pass it on the command line:

```bash
scrn -w ~/projects
```

## Keybindings

**Session list:** `j/k` navigate, `g/G` top/bottom, `Enter` attach, `c` create, `x` kill, `X` kill all, `o` toggle opened filter, `d` go home, `/` search, `r` refresh, `?` legend, `q` quit

**Attached:** `Esc Esc` detach, `Ctrl+S` swap pane, `Ctrl+A,D` standard Screen detach

## Development

```bash
cargo build    # debug build
just build     # release build
just lint      # format + clippy
```
