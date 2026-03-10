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
const PROC_FG: Color = Color::Rgb(160, 190, 140);
const VERSION_FG: Color = Color::Rgb(80, 80, 100);
const COUNT_FG: Color = Color::Rgb(100, 100, 120);
const SECTION_FG: Color = Color::Rgb(140, 120, 180);
const CONST_BG: Color = Color::Rgb(35, 16, 24);
const CONST_ZEBRA_BG: Color = Color::Rgb(48, 22, 33);
const PIN_BG: Color = Color::Rgb(22, 20, 30);
const PIN_ZEBRA_BG: Color = Color::Rgb(33, 30, 46);
const REPO_FG: Color = Color::Rgb(180, 180, 200);
const NOTE_FG: Color = Color::Rgb(200, 175, 110);
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
        Mode::Renaming => {
            dim_background(f);
            draw_rename_modal(f, app);
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
        Mode::EditingNote => {
            dim_background(f);
            draw_note_modal(f, app);
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
    const NOTE_W: u16 = 28;
    const COL_SPACING: u16 = 2;
    const BORDERS: u16 = 2;
    const HIGHLIGHT_SYM: u16 = 2;
    const MIN_NAME_W: u16 = 10;

    // Measure widest actual process text and session name to size columns to content.
    let max_proc_chars = app.display_items.iter().filter_map(|item| match item {
        ListItem::SessionItem(s) => Some(app.session_proc(&s.pid_name).chars().count()),
        ListItem::TreeRepo { session: Some(s), .. } => Some(app.session_proc(&s.pid_name).chars().count()),
        _ => None,
    }).max().unwrap_or(0) as u16;

    let max_name_chars = {
        let mut max = MIN_NAME_W as usize;
        for item in &app.display_items {
            let n = match item {
                ListItem::SessionItem(s) => s.name.chars().count() + 3, // widest prefix " ↳ "
                ListItem::TreeRepo { name, prefix, .. } => name.chars().count() + 2 + prefix.chars().count(),
                _ => continue,
            };
            if n > max { max = n; }
        }
        max as u16
    };

    let fixed_base = NOTE_W + (COL_SPACING * 2) + BORDERS + HIGHLIGHT_SYM;
    let available = area.width.saturating_sub(fixed_base);

    let (name_w, proc_w) = if max_name_chars + max_proc_chars <= available {
        (max_name_chars, max_proc_chars)
    } else {
        let name_priority = (available / 2).max(MIN_NAME_W);
        let pw = max_proc_chars.min(available.saturating_sub(name_priority));
        let nw = available.saturating_sub(pw).max(MIN_NAME_W);
        (nw, pw)
    };

    let used = BORDERS + HIGHLIGHT_SYM + name_w + proc_w + COL_SPACING * 2;
    let note_w = if area.width > used { area.width - used } else { NOTE_W }.max(NOTE_W);

    let name_chars = name_w as usize;

    let header_style = Style::default()
        .fg(HEADER_FG)
        .bg(BASE_BG)
        .add_modifier(Modifier::BOLD);

    let header_cells = vec![
        Cell::from("Name"),
        Cell::from("Process"),
        Cell::from("Note"),
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
        .map(|(_i, item)| match item {
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
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                ];
                Row::new(cells).style(Style::default().fg(SECTION_FG).bg(BASE_BG))
            }
            ListItem::Separator => {
                let line_char = "\u{2500}"; // ─
                let total_w = (name_w + proc_w + note_w + COL_SPACING * 2) as usize;
                let line_str: String = line_char.repeat(total_w);
                let cells = vec![
                    Cell::from(Span::styled(
                        line_str,
                        Style::default().fg(BORDER_FG).bg(BASE_BG),
                    )),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                ];
                Row::new(cells)
                    .style(Style::default().fg(BORDER_FG).bg(BASE_BG))
                    .bottom_margin(1)
            }
            ListItem::TreeDir { name, prefix, .. } => {
                let mut spans = vec![
                    Span::styled("  ", Style::default().fg(SECTION_FG).bg(BASE_BG)),
                ];
                if !prefix.is_empty() {
                    spans.push(Span::styled(
                        prefix.clone(),
                        Style::default().fg(TREE_GUIDE).bg(BASE_BG),
                    ));
                }
                spans.push(Span::styled(
                    format!("{name}/"),
                    Style::default()
                        .fg(SECTION_FG)
                        .bg(BASE_BG)
                        .add_modifier(Modifier::DIM),
                ));
                let cells = vec![
                    Cell::from(Line::from(spans)),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                ];
                Row::new(cells).style(Style::default().fg(SECTION_FG).bg(BASE_BG))
            }
            ListItem::TreeRepo {
                name,
                session,
                companion,
                prefix,
                ..
            } => {
                let const_idx = app.constants.iter().position(|n| n == name);
                let bg = if const_idx.is_some() {
                    if selectable_row_idx % 2 == 1 { CONST_ZEBRA_BG } else { CONST_BG }
                } else if app.pins.contains(name) {
                    if selectable_row_idx % 2 == 1 { PIN_ZEBRA_BG } else { PIN_BG }
                } else if selectable_row_idx % 2 == 1 { ZEBRA_BG } else { BASE_BG };
                selectable_row_idx += 1;

                let hotkey_prefix = const_idx
                    .filter(|&i| i < 9)
                    .map(|i| format!("{} ", i + 1))
                    .unwrap_or_else(|| "  ".to_string());
                let avail = name_chars.saturating_sub(hotkey_prefix.len() + prefix.chars().count());
                let name_text = truncate(name, avail);
                let has_session = session.is_some();
                let name_fg = if has_session { GREEN } else { REPO_FG };

                let name_cell = if !app.search_input.is_empty() {
                    if let Some((positions, _)) = fuzzy_match(name, &app.search_input) {
                        let max_pos = name_text.chars().count();
                        let highlight_set: std::collections::HashSet<usize> =
                            positions.into_iter().filter(|&p| p < max_pos).collect();
                        let mut spans = vec![
                            Span::styled(hotkey_prefix.clone(), Style::default().fg(DIM).bg(bg)),
                            Span::styled(prefix.clone(), Style::default().fg(TREE_GUIDE).bg(bg)),
                        ];
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
                                let style =
                                    if current_is_match { match_style } else { normal_style };
                                spans.push(Span::styled(std::mem::take(&mut current), style));
                            }
                            current.push(ch);
                            current_is_match = is_match;
                        }
                        if !current.is_empty() {
                            let style =
                                if current_is_match { match_style } else { normal_style };
                            spans.push(Span::styled(current, style));
                        }
                        Cell::from(Line::from(spans))
                    } else {
                        let mut spans = vec![
                            Span::styled(hotkey_prefix.clone(), Style::default().fg(DIM).bg(bg)),
                            Span::styled(prefix.clone(), Style::default().fg(TREE_GUIDE).bg(bg)),
                        ];
                        spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                        Cell::from(Line::from(spans))
                    }
                } else {
                    let mut spans = vec![
                        Span::styled(hotkey_prefix.clone(), Style::default().fg(DIM).bg(bg)),
                        Span::styled(prefix.clone(), Style::default().fg(TREE_GUIDE).bg(bg)),
                    ];
                    spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                    Cell::from(Line::from(spans))
                };

                let _ = companion; // companion is shown as a separate selectable row below

                let proc_text = truncate(
                    &session.as_ref().map(|s| app.session_proc(&s.pid_name).to_string()).unwrap_or_default(),
                    proc_w as usize,
                );

                let proc_cell = if proc_text.is_empty() {
                    Cell::from(Span::styled("\u{2500}", Style::default().fg(DIM).bg(bg)))
                } else {
                    Cell::from(Span::styled(proc_text, Style::default().fg(PROC_FG).bg(bg)))
                };

                let note_text = app.notes.get(name)
                    .map(|n| truncate(n, note_w as usize))
                    .unwrap_or_default();

                let cells = vec![
                    name_cell,
                    proc_cell,
                    Cell::from(Span::styled(note_text, Style::default().fg(NOTE_FG).bg(bg))),
                ];
                Row::new(cells).style(Style::default().fg(FG).bg(bg))
            }
            ListItem::SessionItem(session) => {
                let is_current = app.is_current_session(session);
                let is_throwaway = session.name.starts_with("tmp-");

                let const_idx = app.constants.iter().position(|n| n == &session.name);
                let bg = if const_idx.is_some() {
                    if selectable_row_idx % 2 == 1 { CONST_ZEBRA_BG } else { CONST_BG }
                } else if app.pins.contains(&session.name) {
                    if selectable_row_idx % 2 == 1 { PIN_ZEBRA_BG } else { PIN_BG }
                } else if selectable_row_idx % 2 == 1 { ZEBRA_BG } else { BASE_BG };
                selectable_row_idx += 1;
                let is_inactive_const = const_idx.is_some() && session.pid_name.is_empty();
                let is_companion = !is_current && !is_throwaway && session.name.ends_with("-2");
                let name_fg = if is_inactive_const { DIM } else if is_current { ACCENT } else if is_throwaway { DIM } else { GREEN };
                let hotkey_prefix = const_idx
                    .filter(|&i| i < 9)
                    .map(|i| format!("{} ", i + 1));
                let prefix = if let Some(ref hp) = hotkey_prefix {
                    hp.as_str()
                } else if is_current { "\u{25c6} " } else if is_throwaway { "~ " } else if is_companion { " \u{21b3} " } else { "  " };
                let avail = name_chars.saturating_sub(prefix.chars().count());
                let display_name = if name_counts.get(&session.name).copied().unwrap_or(0) > 1 {
                    let seen = name_seen.entry(session.name.clone()).or_insert(0);
                    *seen += 1;
                    if *seen > 1 {
                        format!("{} \u{b7}{}", session.name, seen)
                    } else {
                        session.name.clone()
                    }
                } else {
                    session.name.clone()
                };
                let name_text = truncate(&display_name, avail);

                let prefix_fg = if is_current { ACCENT } else { FG };

                let name_cell = if !app.search_input.is_empty() {
                    if let Some((positions, _)) = fuzzy_match(&session.name, &app.search_input) {
                        let max_pos = name_text.chars().count();
                        let highlight_set: std::collections::HashSet<usize> =
                            positions.into_iter().filter(|&p| p < max_pos).collect();
                        let mut spans = vec![Span::styled(
                            prefix.to_string(),
                            Style::default().fg(prefix_fg).bg(bg),
                        )];
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
                                let style =
                                    if current_is_match { match_style } else { normal_style };
                                spans.push(Span::styled(std::mem::take(&mut current), style));
                            }
                            current.push(ch);
                            current_is_match = is_match;
                        }
                        if !current.is_empty() {
                            let style =
                                if current_is_match { match_style } else { normal_style };
                            spans.push(Span::styled(current, style));
                        }
                        Cell::from(Line::from(spans))
                    } else {
                        let mut spans = vec![
                            Span::styled(
                                prefix.to_string(),
                                Style::default().fg(prefix_fg).bg(bg),
                            ),
                        ];
                        spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                        Cell::from(Line::from(spans))
                    }
                } else {
                    let mut spans = vec![
                        Span::styled(
                            prefix.to_string(),
                            Style::default().fg(prefix_fg).bg(bg),
                        ),
                    ];
                    spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                    Cell::from(Line::from(spans))
                };

                let proc_text = truncate(app.session_proc(&session.pid_name), proc_w as usize);

                let proc_cell = if proc_text.is_empty() {
                    Cell::from(Span::styled("\u{2500}", Style::default().fg(DIM).bg(bg)))
                } else {
                    Cell::from(Span::styled(proc_text, Style::default().fg(PROC_FG).bg(bg)))
                };

                let note_text = app.notes.get(&session.name)
                    .map(|n| truncate(n, note_w as usize))
                    .unwrap_or_default();

                let cells = vec![
                    name_cell,
                    proc_cell,
                    Cell::from(Span::styled(note_text, Style::default().fg(NOTE_FG).bg(bg))),
                ];
                Row::new(cells).style(Style::default().fg(FG).bg(bg))
            }
        })
        .collect();

    let widths_vec = vec![
        Constraint::Length(name_w),
        Constraint::Length(proc_w),
        Constraint::Length(note_w),
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

    let mut hints: Vec<(&str, &str)> = vec![
        ("Enter","Attach"), ("c","Create"), ("t","Throwaway"), ("d","Duplicate"), ("n","Rename"), ("x","Kill"), ("p","Pin"), ("C","Constant"), ("s","Note"), ("/","Search"), ("q","Quit"),
    ];
    if app.workspace_tree.as_ref().is_some_and(|t| t.children.iter().any(|c| !c.is_repo)) {
        hints.push(("O", "Order"));
    }
    if !app.constants.is_empty() {
        hints.push(("R", "Reorder"));
    }
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
                Style::default()
                    .bg(HIGHLIGHT_BG)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("\u{2588} ");

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

// ── Rename modal ────────────────────────────────────────────

fn draw_rename_modal(f: &mut Frame, app: &App) {
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
            " Rename Session ",
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
            " New name:",
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

// ── Note modal ──────────────────────────────────────────────

fn draw_note_modal(f: &mut Frame, app: &App) {
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
            format!(" Note: {name} "),
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
        Line::from(Span::styled(" Note:", Style::default().fg(DIM).bg(MODAL_BG))),
        Line::from(Span::styled(
            format!(" {display}"),
            Style::default().fg(NOTE_FG).bg(MODAL_BG).add_modifier(Modifier::BOLD),
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

