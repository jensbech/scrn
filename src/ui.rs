use std::collections::HashMap;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Table, TableState,
};
use ratatui::Frame;

use crate::app::{fuzzy_match, App, ListItem, Mode};
use crate::screen::Session;


fn pill_label(session_name: &str, labels: &HashMap<String, String>) -> String {
    if let Some(l) = labels.get(session_name) {
        return l.clone();
    }
    for n in 2..=9 {
        if session_name.ends_with(&format!("-{n}")) {
            return n.to_string();
        }
    }
    "1".to_string()
}

fn pills_display_width(pills: &[Session], labels: &HashMap<String, String>) -> usize {
    if pills.is_empty() {
        return 0;
    }
    let mut w = 0;
    for (i, p) in pills.iter().enumerate() {
        let lbl = pill_label(&p.name, labels);
        // "[label]" = 2 brackets + label chars
        w += 2 + lbl.chars().count();
        if i + 1 < pills.len() {
            w += 1; // single-space separator
        }
    }
    w
}


/// Render pills as `[N]` text. Only the row under the cursor shows its active
/// pill in GREEN; everything else is a dim outline. No bg fill anywhere —
/// this keeps the table reading as a single grid rather than a collage.
fn build_pill_spans(
    pills: &[Session],
    active_idx: usize,
    labels: &HashMap<String, String>,
    row_bg: Color,
    row_is_selected: bool,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    if pills.is_empty() {
        return out;
    }
    for (i, p) in pills.iter().enumerate() {
        if i > 0 {
            out.push(Span::styled(" ".to_string(), Style::default().bg(row_bg)));
        }
        let lbl = pill_label(&p.name, labels);
        let is_active = row_is_selected && i == active_idx;
        let fg = if is_active { GREEN } else { DIM };
        let mut modifier = Modifier::empty();
        if is_active {
            modifier |= Modifier::BOLD;
        }
        out.push(Span::styled(
            format!("[{lbl}]"),
            Style::default().fg(fg).bg(row_bg).add_modifier(modifier),
        ));
    }
    out
}

// ── Palette — ALL explicit Rgb, zero ANSI named colors ─────
const BASE_BG: Color = Color::Rgb(18, 18, 24);
const ZEBRA_BG: Color = Color::Rgb(30, 30, 40);
const HIGHLIGHT_BG: Color = Color::Rgb(55, 55, 80);
const DIM: Color = Color::Rgb(100, 100, 110);
const ACCENT: Color = Color::Rgb(180, 180, 255);
const FG: Color = Color::Rgb(220, 220, 230);
const FG_BRIGHT: Color = Color::Rgb(255, 255, 255);
const GREEN: Color = Color::Rgb(80, 200, 120);
const MATCH_FG: Color = Color::Rgb(255, 200, 60);
const SEARCH_BG: Color = Color::Rgb(25, 25, 35);
const MODAL_BG: Color = Color::Rgb(20, 20, 30);
const MODAL_BORDER: Color = Color::Rgb(80, 80, 110);
const MODAL_TITLE: Color = Color::Rgb(180, 180, 200);
const BORDER_FG: Color = Color::Rgb(60, 60, 80);
const HEADER_FG: Color = Color::Rgb(180, 180, 200);
const STATUS_OK: Color = Color::Rgb(140, 220, 140);
const STATUS_ERR: Color = Color::Rgb(220, 140, 140);
const KILL_BORDER: Color = Color::Rgb(200, 80, 80);
const KILL_BG: Color = Color::Rgb(30, 15, 15);
const KILL_TITLE: Color = Color::Rgb(220, 140, 140);
const DIM_FG: Color = Color::Rgb(50, 50, 60);
const DIM_BG: Color = Color::Rgb(10, 10, 15);
const VERSION_FG: Color = Color::Rgb(80, 80, 100);
const COUNT_FG: Color = Color::Rgb(100, 100, 120);
const SECTION_FG: Color = Color::Rgb(140, 120, 180);
const CONST_BG: Color = Color::Rgb(35, 16, 24);
const CONST_ZEBRA_BG: Color = Color::Rgb(48, 22, 33);
const PIN_BG: Color = Color::Rgb(22, 20, 30);
const PIN_ZEBRA_BG: Color = Color::Rgb(33, 30, 46);
const REPO_FG: Color = Color::Rgb(180, 180, 200);
const TREE_GUIDE: Color = Color::Rgb(55, 55, 75);

fn split_at_char_pos(s: &str, pos: usize) -> (&str, &str) {
    let byte_pos = s
        .char_indices()
        .nth(pos)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s.split_at(byte_pos)
}

fn visible_input(input: &str, cursor_pos: usize, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count < max_chars {
        let (before, after) = split_at_char_pos(input, cursor_pos);
        return format!("{before}|{after}");
    }
    let budget = max_chars.saturating_sub(1);
    let half = budget / 2;
    let mut start = cursor_pos.saturating_sub(half);
    let mut end = start + budget;
    if end > char_count {
        end = char_count;
        start = end.saturating_sub(budget);
    }
    let left_ellipsis = start > 0;
    let right_ellipsis = end < char_count;
    if left_ellipsis {
        start += 1;
    }
    if right_ellipsis && end > start {
        end -= 1;
    }
    let visible: String = input.chars().skip(start).take(end - start).collect();
    let cursor_in_vis = cursor_pos.saturating_sub(start);
    let (before, after) = split_at_char_pos(&visible, cursor_in_vis);
    let mut result = String::new();
    if left_ellipsis {
        result.push('\u{2026}');
    }
    result.push_str(before);
    result.push('|');
    result.push_str(after);
    if right_ellipsis {
        result.push('\u{2026}');
    }
    result
}

/// Number of display columns a SessionItem prefix will consume (hotkey / current
/// marker / throwaway marker / companion arrow). Must stay in sync with the
/// renderer below.
fn session_prefix_width(session: &crate::screen::Session, app: &App) -> usize {
    let const_idx = app.constants.iter().position(|n| n == &session.name);
    if const_idx.filter(|&i| i < 9).is_some() {
        return 2;
    }
    if app.is_current_session(session) {
        return 2; // ◆ + space
    }
    if session.name.starts_with("tmp-") {
        return 2; // ~ + space
    }
    let is_companion = (2..=9).any(|n| session.name.ends_with(&format!("-{n}")));
    if is_companion {
        return 5;
    }
    2
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else if max <= 3 {
        s.chars().take(max).collect()
    } else {
        let t: String = s.chars().take(max - 1).collect();
        format!("{t}\u{2026}")
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    // Paint entire screen with explicit fg + bg on every cell.
    let area = f.area();
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(FG);
                cell.set_bg(BASE_BG);
                cell.set_symbol(" ");
            }
        }
    }

    let show_search_bar = app.mode == Mode::Searching || !app.search_input.is_empty();
    let constraints = if show_search_bar {
        vec![Constraint::Min(3), Constraint::Length(1)]
    } else {
        vec![Constraint::Min(0)]
    };
    let chunks = Layout::vertical(constraints).split(f.area());

    const MAX_WIDTH: u16 = 140;
    let table_area = if chunks[0].width > MAX_WIDTH {
        let x = chunks[0].x + (chunks[0].width - MAX_WIDTH) / 2;
        Rect::new(x, chunks[0].y, MAX_WIDTH, chunks[0].height)
    } else {
        chunks[0]
    };
    draw_table(f, app, table_area);
    if show_search_bar {
        draw_search_bar(f, app, chunks[1]);
    }

    match app.mode {
        Mode::Creating => {
            dim_background(f);
            draw_create_modal(f, app);
        }
        Mode::ConfirmPin => {
            dim_background(f);
            draw_pin_modal(f, app);
        }
        Mode::ConfirmConstant => {
            dim_background(f);
            draw_constant_modal(f, app);
        }
        Mode::ConfirmKill => {
            dim_background(f);
            draw_kill_modal(f, app);
        }
        Mode::ConfirmKillAll1 => {
            dim_background(f);
            draw_kill_all_modal_1(f, app);
        }
        Mode::ConfirmKillAll2 => {
            dim_background(f);
            draw_kill_all_modal_2(f);
        }
        Mode::ConfirmQuit => {
            dim_background(f);
            draw_quit_modal(f);
        }
        Mode::Ordering => {
            dim_background(f);
            draw_ordering_modal(f, app);
        }
        Mode::ConstantOrdering => {
            dim_background(f);
            draw_constant_ordering_modal(f, app);
        }
        Mode::EditingCommand => {
            dim_background(f);
            draw_command_modal(f, app);
        }
        Mode::EditingLabel => {
            dim_background(f);
            draw_label_modal(f, app, false);
        }
        Mode::LabelNewCompanion => {
            dim_background(f);
            draw_label_modal(f, app, true);
        }
        _ => {}
    }

}

fn dim_background(f: &mut Frame) {
    let area = f.area();
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(DIM_FG);
                cell.set_bg(DIM_BG);
            }
        }
    }
}

// ── Main table ──────────────────────────────────────────────

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    const COL_SPACING: u16 = 2;
    const BORDERS: u16 = 2;
    const MIN_NAME_W: u16 = 10;
    const MAX_DIR_NAME_CHARS: usize = 24;

    // Name column width = longest name+prefix across all selectable rows.
    // Folder names are capped; repos/sessions always show in full so custom
    // labels and repo names have the room they need.
    let max_name_chars = {
        let mut max = MIN_NAME_W as usize;
        for item in &app.display_items {
            let n = match item {
                ListItem::SessionItem(s) => session_prefix_width(s, app) + s.name.chars().count(),
                ListItem::TreeRepo { name, prefix, .. } => {
                    2 + prefix.chars().count() + name.chars().count()
                }
                ListItem::TreeDir { name, descendant_repos, descendant_open, folded, .. } => {
                    let name_w = name.chars().count().min(MAX_DIR_NAME_CHARS);
                    let base = 2 + name_w + 1;
                    if *folded {
                        base + 2 + format!("{descendant_repos}").chars().count()
                            + if *descendant_open > 0 { format!(" ({descendant_open} open)").chars().count() } else { 0 }
                    } else {
                        base
                    }
                }
                _ => continue,
            };
            if n > max { max = n; }
        }
        max as u16
    };

    // Tabs column: widest pill-strip across TreeRepo rows.
    let tabs_w = {
        let mut max = 0usize;
        for item in &app.display_items {
            if let ListItem::TreeRepo { pills, .. } = item {
                let w = pills_display_width(pills, &app.companion_labels);
                if w > max { max = w; }
            }
        }
        (max as u16).max(1)
    };

    let fixed_base = (COL_SPACING * 2) + BORDERS;
    let available = area.width.saturating_sub(fixed_base);

    let want = max_name_chars + tabs_w;
    let (name_w, tabs_col_w) = if want <= available {
        (max_name_chars, tabs_w)
    } else {
        let nw = available.saturating_sub(tabs_w).max(MIN_NAME_W);
        (nw, tabs_w)
    };

    let name_chars = name_w as usize;
    let name_align_width = (max_name_chars as usize).min(name_chars);

    let header_style = Style::default()
        .fg(HEADER_FG)
        .bg(BASE_BG)
        .add_modifier(Modifier::BOLD);

    let header_cells = vec![
        Cell::from("Name"),
        Cell::from("Tabs"),
    ];
    let header = Row::new(header_cells)
    .style(header_style)
    .bottom_margin(1);

    let selected_visual_row = app
        .selectable_indices
        .get(app.selected)
        .copied();

    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for item in &app.display_items {
        if let ListItem::SessionItem(s) = item {
            *name_counts.entry(s.name.clone()).or_insert(0) += 1;
        }
    }
    let mut name_seen: HashMap<String, usize> = HashMap::new();

    let mut selectable_row_idx = 0usize;
    let rows: Vec<Row> = app
        .display_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = Some(i) == selected_visual_row;
            match item {
            ListItem::SectionHeader(title) => {
                let full_width_name = format!(" \u{2500} {title}");
                let cells = vec![
                    Cell::from(Line::from(Span::styled(
                        full_width_name,
                        Style::default()
                            .fg(SECTION_FG)
                            .bg(BASE_BG)
                            .add_modifier(Modifier::BOLD | Modifier::DIM),
                    ))),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                ];
                Row::new(cells).style(Style::default().fg(SECTION_FG).bg(BASE_BG))
            }
            ListItem::Separator => {
                let line_char = "\u{2500}";
                let total_w = (name_w + tabs_col_w + COL_SPACING) as usize;
                let line_str: String = line_char.repeat(total_w);
                let cells = vec![
                    Cell::from(Span::styled(
                        line_str,
                        Style::default().fg(BORDER_FG).bg(BASE_BG),
                    )),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                ];
                Row::new(cells)
                    .style(Style::default().fg(BORDER_FG).bg(BASE_BG))
                    .bottom_margin(1)
            }
            ListItem::TreeDir { name, prefix, folded, descendant_repos, descendant_open, .. } => {
                let base_bg = if selectable_row_idx % 2 == 1 { ZEBRA_BG } else { BASE_BG };
                let bg = if is_selected { HIGHLIGHT_BG } else { base_bg };
                selectable_row_idx += 1;

                let icon = if *folded { "\u{25B8} " } else { "\u{25BE} " };
                let icon_fg = if *folded { ACCENT } else { SECTION_FG };
                let mut spans = vec![
                    Span::styled(icon, Style::default().fg(icon_fg).bg(bg)),
                ];
                if !prefix.is_empty() {
                    spans.push(Span::styled(
                        prefix.clone(),
                        Style::default().fg(TREE_GUIDE).bg(bg),
                    ));
                }
                let truncated = truncate(name, MAX_DIR_NAME_CHARS);
                spans.push(Span::styled(
                    format!("{truncated}/"),
                    Style::default()
                        .fg(SECTION_FG)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ));
                if *folded && *descendant_repos > 0 {
                    spans.push(Span::styled(
                        format!("  {descendant_repos}"),
                        Style::default().fg(DIM).bg(bg),
                    ));
                    if *descendant_open > 0 {
                        spans.push(Span::styled(
                            format!(" ({descendant_open} open)"),
                            Style::default().fg(GREEN).bg(bg).add_modifier(Modifier::DIM),
                        ));
                    }
                }
                let cells = vec![
                    Cell::from(Line::from(spans)),
                    Cell::from(Span::styled("", Style::default().bg(bg))),
                ];
                Row::new(cells).style(Style::default().fg(SECTION_FG).bg(bg))
            }
            ListItem::TreeRepo {
                name,
                pills,
                active_idx,
                prefix,
                ..
            } => {
                let const_idx = app.constants.iter().position(|n| n == name);
                let base_bg = if const_idx.is_some() {
                    if selectable_row_idx % 2 == 1 { CONST_ZEBRA_BG } else { CONST_BG }
                } else if app.pins.contains(name) {
                    if selectable_row_idx % 2 == 1 { PIN_ZEBRA_BG } else { PIN_BG }
                } else if selectable_row_idx % 2 == 1 { ZEBRA_BG } else { BASE_BG };
                let bg = if is_selected { HIGHLIGHT_BG } else { base_bg };
                selectable_row_idx += 1;

                let hotkey_prefix = const_idx
                    .filter(|&i| i < 9)
                    .map(|i| format!("{} ", i + 1))
                    .unwrap_or_else(|| "  ".to_string());
                let display_prefix = if const_idx.is_some() { "" } else { prefix.as_str() };
                let used_prefix = hotkey_prefix.chars().count() + display_prefix.chars().count();
                let has_session = !pills.is_empty();
                let name_fg = if has_session { GREEN } else { REPO_FG };

                let max_name_avail = name_chars.saturating_sub(used_prefix);
                let name_text = truncate(name, max_name_avail);

                let mut spans: Vec<Span> = Vec::new();
                spans.push(Span::styled(hotkey_prefix.clone(), Style::default().fg(DIM).bg(bg)));
                spans.push(Span::styled(display_prefix, Style::default().fg(TREE_GUIDE).bg(bg)));

                if !app.search_input.is_empty() {
                    if let Some((positions, _)) = fuzzy_match(name, &app.search_input) {
                        let max_pos = name_text.chars().count();
                        let highlight_set: std::collections::HashSet<usize> =
                            positions.into_iter().filter(|&p| p < max_pos).collect();
                        let normal_style = Style::default().fg(name_fg).bg(bg);
                        let match_style = Style::default()
                            .fg(MATCH_FG)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD);
                        let mut current = String::new();
                        let mut current_is_match = false;
                        for (ci, ch) in name_text.chars().enumerate() {
                            let is_match = highlight_set.contains(&ci);
                            if is_match != current_is_match && !current.is_empty() {
                                let style = if current_is_match { match_style } else { normal_style };
                                spans.push(Span::styled(std::mem::take(&mut current), style));
                            }
                            current.push(ch);
                            current_is_match = is_match;
                        }
                        if !current.is_empty() {
                            let style = if current_is_match { match_style } else { normal_style };
                            spans.push(Span::styled(current, style));
                        }
                    } else {
                        spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                    }
                } else {
                    spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                }

                let _ = name_align_width;

                let tab_spans = build_pill_spans(pills, *active_idx, &app.companion_labels, bg, is_selected);

                let cells = vec![
                    Cell::from(Line::from(spans)),
                    Cell::from(Line::from(tab_spans)),
                ];
                Row::new(cells).style(Style::default().fg(FG).bg(bg))
            }
            ListItem::SessionItem(session) => {
                let is_current = app.is_current_session(session);
                let is_throwaway = session.name.starts_with("tmp-");

                let const_idx = app.constants.iter().position(|n| n == &session.name);
                let base_bg = if const_idx.is_some() {
                    if selectable_row_idx % 2 == 1 { CONST_ZEBRA_BG } else { CONST_BG }
                } else if app.pins.contains(&session.name) {
                    if selectable_row_idx % 2 == 1 { PIN_ZEBRA_BG } else { PIN_BG }
                } else if selectable_row_idx % 2 == 1 { ZEBRA_BG } else { BASE_BG };
                let bg = if is_selected { HIGHLIGHT_BG } else { base_bg };
                selectable_row_idx += 1;
                let is_inactive_const = const_idx.is_some() && session.pid_name.is_empty();
                let is_companion = !is_current && !is_throwaway && (2..=9).any(|n| session.name.ends_with(&format!("-{n}")));
                let name_fg = if is_inactive_const { DIM } else if is_current { ACCENT } else if is_throwaway { DIM } else { GREEN };
                let hotkey_prefix = const_idx
                    .filter(|&i| i < 9)
                    .map(|i| format!("{} ", i + 1));
                let prefix = if let Some(ref hp) = hotkey_prefix {
                    hp.as_str()
                } else if is_current { "\u{25c6} " } else if is_throwaway { "~ " } else if is_companion { "   \u{21b3} " } else { "  " };
                let max_name_avail = name_chars.saturating_sub(prefix.chars().count());
                // Prefer the user-facing label over the internal screen name.
                let base_name = app
                    .companion_labels
                    .get(&session.name)
                    .cloned()
                    .unwrap_or_else(|| session.name.clone());
                let display_name = if name_counts.get(&session.name).copied().unwrap_or(0) > 1 {
                    let seen = name_seen.entry(session.name.clone()).or_insert(0);
                    *seen += 1;
                    if *seen > 1 {
                        format!("{} \u{b7}{}", base_name, seen)
                    } else {
                        base_name
                    }
                } else {
                    base_name
                };
                let name_text = truncate(&display_name, max_name_avail);

                let prefix_fg = if is_current { ACCENT } else { FG };

                let mut spans: Vec<Span> = vec![Span::styled(
                    prefix.to_string(),
                    Style::default().fg(prefix_fg).bg(bg),
                )];

                if !app.search_input.is_empty() {
                    if let Some((positions, _)) = fuzzy_match(&session.name, &app.search_input) {
                        let max_pos = name_text.chars().count();
                        let highlight_set: std::collections::HashSet<usize> =
                            positions.into_iter().filter(|&p| p < max_pos).collect();
                        let normal_style = Style::default().fg(name_fg).bg(bg);
                        let match_style = Style::default()
                            .fg(MATCH_FG)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD);
                        let mut current = String::new();
                        let mut current_is_match = false;
                        for (ci, ch) in name_text.chars().enumerate() {
                            let is_match = highlight_set.contains(&ci);
                            if is_match != current_is_match && !current.is_empty() {
                                let style = if current_is_match { match_style } else { normal_style };
                                spans.push(Span::styled(std::mem::take(&mut current), style));
                            }
                            current.push(ch);
                            current_is_match = is_match;
                        }
                        if !current.is_empty() {
                            let style = if current_is_match { match_style } else { normal_style };
                            spans.push(Span::styled(current, style));
                        }
                    } else {
                        spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                    }
                } else {
                    spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                }

                let cells = vec![
                    Cell::from(Line::from(spans)),
                    Cell::from(Span::styled("", Style::default().bg(bg))),
                ];
                Row::new(cells).style(Style::default().fg(FG).bg(bg))
            }
        }
        })
        .collect();

    let widths_vec = vec![
        Constraint::Length(name_w),
        Constraint::Length(tabs_col_w),
    ];

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_FG).bg(BASE_BG))
        .style(Style::default().fg(FG).bg(BASE_BG))
        .title(Line::from(vec![
            Span::styled(
                " scrn ",
                Style::default()
                    .fg(ACCENT)
                    .bg(BASE_BG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("v{} ", env!("CARGO_PKG_VERSION")),
                Style::default().fg(VERSION_FG).bg(BASE_BG),
            ),
        ]))
        ;

    // Bottom border: key hints, status message, and/or filter indicator
    let mut bottom_left_spans: Vec<Span> = Vec::new();

    let on_constant = app.selected_item_name()
        .map(|n| app.constants.contains(&n))
        .unwrap_or(false);

    let on_dir = matches!(app.selected_display_item(), Some(ListItem::TreeDir { .. }));

    let mut hints: Vec<(&str, &str)> = Vec::new();
    hints.push(("\u{23ce}","Attach"));
    if on_dir {
        hints.push(("h/l","Fold"));
        hints.push(("z","FoldAll"));
    } else {
        hints.push(("d","Dup+label"));
        hints.push(("\u{2190}/\u{2192}","Pill"));
        hints.push(("Tab","Cycle"));
        hints.push(("`","Back"));
    }
    hints.push(("/","Search"));
    hints.push(("c","New"));
    hints.push(("x","Kill"));
    hints.push(("p","Pin"));
    hints.push(("C","Const"));
    if on_constant {
        hints.push(("e", "Cmd"));
    }
    if !on_dir {
        hints.push(("L","Label"));
    }
    hints.push(("n","Rename"));
    if app.workspace_tree.as_ref().is_some_and(|t| t.children.iter().any(|c| !c.is_repo)) {
        hints.push(("O", "Order"));
    }
    if !app.constants.is_empty() {
        hints.push(("R", "Reorder"));
    }
    hints.push(("q","Quit"));

    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            bottom_left_spans.push(Span::styled(" ", Style::default().fg(DIM).bg(BASE_BG)));
        }
        bottom_left_spans.push(Span::styled(
            format!("{key}"),
            Style::default().fg(ACCENT).bg(BASE_BG),
        ));
        bottom_left_spans.push(Span::styled(
            format!(" {desc}"),
            Style::default().fg(DIM).bg(BASE_BG),
        ));
    }
    block = block.title_bottom(Line::from(bottom_left_spans));

    let mut bottom_right_spans: Vec<Span> = Vec::new();
    if app.filter_opened {
        bottom_right_spans.push(Span::styled(
            " Showing: opened only ",
            Style::default().fg(MATCH_FG).bg(BASE_BG),
        ));
    }
    if !app.status_msg.is_empty() {
        let is_error = app.status_msg.starts_with("Error");
        let fg = if is_error { STATUS_ERR } else { STATUS_OK };
        if !bottom_right_spans.is_empty() {
            bottom_right_spans.push(Span::styled(" ", Style::default().bg(BASE_BG)));
        }
        bottom_right_spans.push(Span::styled(
            format!(" {} ", app.status_msg),
            Style::default().fg(fg).bg(BASE_BG),
        ));
    }
    if !bottom_right_spans.is_empty() {
        block = block.title_bottom(Line::from(bottom_right_spans).right_aligned());
    }

    if app.selectable_indices.is_empty() {
        let msg = if !app.search_input.is_empty() {
            "  No matches"
        } else {
            "  No screen sessions found. Press 'c' to create one."
        };
        let empty_rows: Vec<Row> = vec![Row::new(vec![Cell::from(Span::styled(
            msg,
            Style::default().fg(DIM).bg(BASE_BG),
        ))])
        .style(Style::default().fg(DIM).bg(BASE_BG))];
        let table = Table::new(empty_rows, widths_vec)
            .header(header)
            .block(block)
            .style(Style::default().fg(FG).bg(BASE_BG))
            .column_spacing(COL_SPACING);
        f.render_widget(table, area);
    } else {
        let table = Table::new(rows, widths_vec)
            .header(header)
            .block(block)
            .style(Style::default().fg(FG).bg(BASE_BG))
            .column_spacing(COL_SPACING)
            .row_highlight_style(
                Style::default().add_modifier(Modifier::BOLD),
            );

        // borders(2) + header(1) + header bottom_margin(1) = 4
        let visible_rows = area.height.saturating_sub(4) as usize;

        let mut state = TableState::default();
        if let Some(sel_row) = selected_visual_row {
            let total_rows = app.display_items.len();
            let half = visible_rows / 2;
            let offset = sel_row
                .saturating_sub(half)
                .min(total_rows.saturating_sub(visible_rows));
            *state.offset_mut() = offset;
        }
        state.select(selected_visual_row);

        f.render_stateful_widget(table, area, &mut state);

        app.table_data_y = area.y + 3;
        app.table_data_end_y = area.y + area.height.saturating_sub(1);
        app.table_scroll_offset = state.offset();

        // Scrollbar
        if app.selectable_indices.len() > visible_rows {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(ACCENT))
                .track_style(Style::default().fg(Color::Rgb(40, 40, 60)));
            let mut scrollbar_state = ScrollbarState::new(app.selectable_indices.len())
                .position(app.selected);
            f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
        }
    }
}

// ── Search bar ──────────────────────────────────────────────

fn draw_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let cursor = if app.mode == Mode::Searching {
        "\u{2502}"
    } else {
        ""
    };
    let input_fg = if app.search_filter_active { FG_BRIGHT } else { DIM };
    let mut spans = vec![
        Span::styled(
            " /",
            Style::default()
                .fg(MATCH_FG)
                .bg(SEARCH_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.search_input.clone(),
            Style::default().fg(input_fg).bg(SEARCH_BG),
        ),
        Span::styled(
            cursor.to_string(),
            Style::default().fg(MATCH_FG).bg(SEARCH_BG),
        ),
    ];
    if !app.search_filter_active {
        spans.push(Span::styled(
            " [off]",
            Style::default().fg(DIM).bg(SEARCH_BG),
        ));
    }
    spans.push(Span::styled(
        format!("  ({} matches)", app.selectable_indices.len()),
        Style::default().fg(COUNT_FG).bg(SEARCH_BG),
    ));
    let line = Line::from(spans);

    f.render_widget(
        Paragraph::new(line).style(Style::default().fg(FG).bg(SEARCH_BG)),
        area,
    );
}


// ── Create modal ────────────────────────────────────────────

fn draw_create_modal(f: &mut Frame, app: &App) {
    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            " New Session ",
            Style::default()
                .fg(MODAL_TITLE)
                .bg(MODAL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let max_chars = inner.width.saturating_sub(2) as usize;
    let display = visible_input(&app.create_input, app.cursor_pos, max_chars);

    let lines = vec![
        Line::from(Span::styled(
            " Session name:",
            Style::default().fg(DIM).bg(MODAL_BG),
        )),
        Line::from(Span::styled(
            format!(" {display}"),
            Style::default().fg(FG_BRIGHT).bg(MODAL_BG),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}

// ── Pin confirmation modal ──────────────────────────────────

fn draw_pin_modal(f: &mut Frame, app: &App) {
    let name = app.pin_target.as_deref().unwrap_or("");
    let is_pinned = app.pins.contains(name);
    let action = if is_pinned { "Unpin" } else { "Pin" };

    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            format!(" {action} "),
            Style::default()
                .fg(MODAL_TITLE)
                .bg(MODAL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            format!(" {action} '{name}'?"),
            Style::default().fg(FG_BRIGHT).bg(MODAL_BG),
        )),
        Line::from(Span::styled(
            " y/Enter: confirm  n/Esc: cancel",
            Style::default().fg(DIM).bg(MODAL_BG),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}

// ── Constant confirmation modal ──────────────────────────────

fn draw_constant_modal(f: &mut Frame, app: &App) {
    let name = app.constant_target.as_deref().unwrap_or("");

    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            " Constant ",
            Style::default()
                .fg(MODAL_TITLE)
                .bg(MODAL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            format!(" Add/Remove '{name}' from constants?"),
            Style::default().fg(FG_BRIGHT).bg(MODAL_BG),
        )),
        Line::from(Span::styled(
            " y/Enter: confirm  n/Esc: cancel",
            Style::default().fg(DIM).bg(MODAL_BG),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}

// ── Kill confirmation modal ─────────────────────────────────

fn draw_kill_modal(f: &mut Frame, app: &App) {
    let session_name = app
        .kill_session_info
        .as_ref()
        .map(|(name, _)| name.clone())
        .unwrap_or_default();

    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(KILL_BORDER).bg(KILL_BG))
        .style(Style::default().fg(FG).bg(KILL_BG))
        .title(Span::styled(
            " Kill Session ",
            Style::default()
                .fg(KILL_TITLE)
                .bg(KILL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            format!(" Kill '{session_name}'?"),
            Style::default().fg(FG_BRIGHT).bg(KILL_BG),
        )),
        Line::from(Span::styled(
            " y: confirm  Esc: cancel",
            Style::default().fg(DIM).bg(KILL_BG),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(KILL_BG)),
        inner,
    );
}

// ── Kill-all confirmation modals ─────────────────────────────

fn draw_kill_all_modal_1(f: &mut Frame, app: &App) {
    let count = app.all_sessions.len();
    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(KILL_BORDER).bg(KILL_BG))
        .style(Style::default().fg(FG).bg(KILL_BG))
        .title(Span::styled(
            " Kill All Sessions ",
            Style::default()
                .fg(KILL_TITLE)
                .bg(KILL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            format!(" Kill all {count} sessions?"),
            Style::default().fg(FG_BRIGHT).bg(KILL_BG),
        )),
        Line::from(Span::styled(
            " y/Enter: confirm  n/Esc: cancel",
            Style::default().fg(DIM).bg(KILL_BG),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(KILL_BG)),
        inner,
    );
}

fn draw_kill_all_modal_2(f: &mut Frame) {
    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(KILL_BORDER).bg(KILL_BG))
        .style(Style::default().fg(FG).bg(KILL_BG))
        .title(Span::styled(
            " Are you sure? ",
            Style::default()
                .fg(KILL_TITLE)
                .bg(KILL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            " This cannot be undone.",
            Style::default().fg(FG_BRIGHT).bg(KILL_BG),
        )),
        Line::from(Span::styled(
            " y/Enter: kill all  n/Esc: cancel",
            Style::default().fg(DIM).bg(KILL_BG),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(KILL_BG)),
        inner,
    );
}

// ── Quit confirmation modal ──────────────────────────────────

fn draw_quit_modal(f: &mut Frame) {
    let area = f.area();
    let width = 40u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            " Quit ",
            Style::default()
                .fg(MODAL_TITLE)
                .bg(MODAL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            " Quit scrn?",
            Style::default().fg(FG_BRIGHT).bg(MODAL_BG),
        )),
        Line::from(Span::styled(
            " y/Enter: confirm  n/Esc: cancel",
            Style::default().fg(DIM).bg(MODAL_BG),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}

fn draw_command_modal(f: &mut Frame, app: &App) {
    let name = app.selected_item_name().unwrap_or_default();
    let area = f.area();
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            format!(" Command: {name} "),
            Style::default().fg(MODAL_TITLE).bg(MODAL_BG).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " Enter save  Esc cancel  Backspace clear ",
            Style::default().fg(DIM).bg(MODAL_BG),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let max_chars = inner.width.saturating_sub(2) as usize;
    let display = visible_input(&app.create_input, app.cursor_pos, max_chars);

    let lines = vec![
        Line::from(Span::styled(" Run on open:", Style::default().fg(DIM).bg(MODAL_BG))),
        Line::from(Span::styled(
            format!(" {display}"),
            Style::default().fg(ACCENT).bg(MODAL_BG).add_modifier(Modifier::BOLD),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}

// ── Directory order modal ────────────────────────────────────

fn draw_ordering_modal(f: &mut Frame, app: &App) {
    let area = f.area();
    let n = app.ordering_items.len() as u16;
    let height = (n * 2 + 3).min(area.height.saturating_sub(4));
    let width = app.ordering_items.iter()
        .map(|s| s.chars().count() as u16)
        .max()
        .unwrap_or(20)
        .max(28)
        .saturating_add(10)
        .min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            " Order Directories ",
            Style::default().fg(MODAL_TITLE).bg(MODAL_BG).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " K\u{2191} J\u{2193} move  Enter save  Esc cancel ",
            Style::default().fg(DIM).bg(MODAL_BG),
        )));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, name) in app.ordering_items.iter().enumerate() {
        let selected = i == app.ordering_selected;
        let bg = if selected { HIGHLIGHT_BG } else { MODAL_BG };
        let fg = if selected { FG_BRIGHT } else { FG };
        let prefix = if selected { "  \u{2588} " } else { "    " };
        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(ACCENT).bg(bg)),
            Span::styled(name.clone(), Style::default().fg(fg).bg(bg)),
        ]));
        lines.push(Line::from(""));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}

fn draw_constant_ordering_modal(f: &mut Frame, app: &App) {
    let area = f.area();
    let n = app.ordering_items.len() as u16;
    let height = (n * 2 + 3).min(area.height.saturating_sub(4));
    let width = app.ordering_items.iter()
        .map(|s| s.chars().count() as u16)
        .max()
        .unwrap_or(20)
        .max(30)
        .saturating_add(12)
        .min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            " Order Constants ",
            Style::default().fg(MODAL_TITLE).bg(MODAL_BG).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " K\u{2191} J\u{2193} move  Enter save  Esc cancel ",
            Style::default().fg(DIM).bg(MODAL_BG),
        )));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, name) in app.ordering_items.iter().enumerate() {
        let selected = i == app.ordering_selected;
        let bg = if selected { HIGHLIGHT_BG } else { MODAL_BG };
        let fg = if selected { FG_BRIGHT } else { FG };
        let num = if i < 9 { format!("{}", i + 1) } else { " ".to_string() };
        let prefix = if selected { format!(" {} \u{2588} ", num) } else { format!(" {}   ", num) };
        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(if selected { ACCENT } else { DIM }).bg(bg)),
            Span::styled(name.clone(), Style::default().fg(fg).bg(bg)),
        ]));
        lines.push(Line::from(""));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}



fn draw_label_modal(f: &mut Frame, app: &App, creating: bool) {
    let title = if creating { " Name new companion " } else { " Rename " };
    let hint_line_top = if creating {
        " Label this companion (optional):"
    } else {
        " Label:"
    };

    let area = f.area();
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 5u16;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            title,
            Style::default().fg(MODAL_TITLE).bg(MODAL_BG).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " Enter save  Esc cancel  Backspace clear ",
            Style::default().fg(DIM).bg(MODAL_BG),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let max_chars = inner.width.saturating_sub(2) as usize;
    let display = visible_input(&app.create_input, app.cursor_pos, max_chars);

    let lines = vec![
        Line::from(Span::styled(hint_line_top, Style::default().fg(DIM).bg(MODAL_BG))),
        Line::from(Span::styled(
            format!(" {display}"),
            Style::default().fg(FG_BRIGHT).bg(MODAL_BG).add_modifier(Modifier::BOLD),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}
