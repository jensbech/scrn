# MRU Quick-Switch Overlay

## Problem

When working across many repos, users must visually scan the full list to find a recently visited repo. Mixed casing adds friction. A quick-switch overlay for the 5 most recent repos removes the need to scan.

## Design

### Trigger

`r` key — not currently bound. Pressing `r` again or `Esc` dismisses without action.

### Overlay

A small centered floating modal rendered on top of the existing list (consistent with create/rename/kill modals). Width sized to the widest repo name plus padding, capped at ~40 chars. Height: up to 5 entries plus a title row.

### Content

- Title: `recent` (dimmed header, same style as other modals)
- One repo/session per row, repo name only — no branch/note/process columns
- Active session indicated by green color on the name (same as main list)
- Most recent entry at top, selection starts there

### Navigation

- `j`/`k` or arrow keys: move selection
- `Enter`: attach or create session (same logic as main list `select_for_attach`)
- `Esc` or `r`: dismiss, return to main list with original selection restored

### Data Source

Constants (from `App.constants`) appear first, sorted alphabetically, regardless of recency. Then the 5 most-recent non-constant entries from the existing `~/.config/scrn/history` file (sorted by timestamp descending). Each name is looked up against the current session list to determine color.

### State

A new `Modal::Recent { selected: usize }` variant added to the existing `Modal` enum in `app.rs`. Cleared on dismiss or attach.

## Out of Scope

- Numbers/shortcuts for direct jump
- Search within the MRU list
- Configurable MRU size (hardcoded 5)
