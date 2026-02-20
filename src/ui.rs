use std::io::Write;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Table, TableState,
};
use ratatui::Frame;

use crate::app::{fuzzy_match, App, ListItem, Mode, Pane, SidebarFocus};
use crate::screen::SessionState;

// ── Palette — ALL explicit Rgb, zero ANSI named colors ─────
const BASE_BG: Color = Color::Rgb(18, 18, 24);
const ZEBRA_BG: Color = Color::Rgb(30, 30, 40);
const HIGHLIGHT_BG: Color = Color::Rgb(55, 55, 80);
const DIM: Color = Color::Rgb(100, 100, 110);
const ACCENT: Color = Color::Rgb(180, 180, 255);
const FG: Color = Color::Rgb(220, 220, 230);
const FG_BRIGHT: Color = Color::Rgb(255, 255, 255);
const GREEN: Color = Color::Rgb(80, 200, 120);
const YELLOW: Color = Color::Rgb(220, 200, 80);
const MATCH_FG: Color = Color::Rgb(255, 200, 60);
const SEARCH_BG: Color = Color::Rgb(25, 25, 35);
const MODAL_BG: Color = Color::Rgb(20, 20, 30);
const MODAL_BORDER: Color = Color::Rgb(80, 80, 110);
const MODAL_TITLE: Color = Color::Rgb(180, 180, 200);
const BORDER_FG: Color = Color::Rgb(60, 60, 80);
const HEADER_FG: Color = Color::Rgb(180, 180, 200);
const HELP_FG: Color = Color::Rgb(120, 120, 140);
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
const REPO_FG: Color = Color::Rgb(180, 180, 200);
const SEPARATOR_FG: Color = Color::Rgb(60, 60, 80);
const ACTIVE_PANE_BORDER: Color = Color::Rgb(120, 160, 255);
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

pub fn draw(f: &mut Frame, app: &App) {
    // Sidebar mode: always use sidebar layout (both list and attached views)
    if app.sidebar_mode {
        draw_sidebar_layout(f, app);
        return;
    }

    // Attached mode: render the embedded PTY terminal
    if app.mode == Mode::Attached {
        draw_attached(f, app);
        return;
    }

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

    draw_table(f, app, chunks[0]);
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
        _ => {}
    }

    if app.show_legend {
        draw_legend(f, app);
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

fn draw_table(f: &mut Frame, app: &App, area: Rect) {
    const STATE_W: u16 = 10;
    const DATE_W: u16 = 22;
    const COL_SPACING: u16 = 2;
    const BORDERS: u16 = 2;
    const HIGHLIGHT_SYM: u16 = 2;

    let fixed = STATE_W + DATE_W + (COL_SPACING * 2) + BORDERS + HIGHLIGHT_SYM;
    let name_w = area.width.saturating_sub(fixed).max(10);
    let name_chars = name_w as usize;

    let header_style = Style::default()
        .fg(HEADER_FG)
        .bg(BASE_BG)
        .add_modifier(Modifier::BOLD);

    let header = Row::new(vec![
        Cell::from("Name"),
        Cell::from("State"),
        Cell::from("Last opened"),
    ])
    .style(header_style)
    .bottom_margin(1);

    let selected_visual_row = app
        .selectable_indices
        .get(app.selected)
        .copied();

    let mut selectable_row_idx = 0usize;
    let rows: Vec<Row> = app
        .display_items
        .iter()
        .enumerate()
        .map(|(_i, item)| match item {
            ListItem::SectionHeader(title) => {
                let full_width_name = format!("  {title}");
                Row::new(vec![
                    Cell::from(Line::from(Span::styled(
                        full_width_name,
                        Style::default()
                            .fg(SECTION_FG)
                            .bg(BASE_BG)
                            .add_modifier(Modifier::BOLD | Modifier::DIM),
                    ))),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                ])
                .style(Style::default().fg(SECTION_FG).bg(BASE_BG))
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
                Row::new(vec![
                    Cell::from(Line::from(spans)),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                    Cell::from(Span::styled("", Style::default().bg(BASE_BG))),
                ])
                .style(Style::default().fg(SECTION_FG).bg(BASE_BG))
            }
            ListItem::TreeRepo {
                name,
                session,
                prefix,
                ..
            } => {
                let bg = if selectable_row_idx % 2 == 1 { ZEBRA_BG } else { BASE_BG };
                selectable_row_idx += 1;

                let margin = "  ";
                let avail = name_chars.saturating_sub(margin.len() + prefix.chars().count());
                let name_text = truncate(name, avail);
                let has_session = session.is_some();
                let name_fg = if has_session { GREEN } else { REPO_FG };

                let name_cell = if !app.search_input.is_empty() {
                    if let Some((positions, _)) = fuzzy_match(name, &app.search_input) {
                        let max_pos = name_text.chars().count();
                        let highlight_set: std::collections::HashSet<usize> =
                            positions.into_iter().filter(|&p| p < max_pos).collect();
                        let mut spans = vec![
                            Span::styled(margin.to_string(), Style::default().fg(name_fg).bg(bg)),
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
                            Span::styled(margin.to_string(), Style::default().fg(name_fg).bg(bg)),
                            Span::styled(prefix.clone(), Style::default().fg(TREE_GUIDE).bg(bg)),
                        ];
                        spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                        Cell::from(Line::from(spans))
                    }
                } else {
                    let mut spans = vec![
                        Span::styled(margin.to_string(), Style::default().fg(name_fg).bg(bg)),
                        Span::styled(prefix.clone(), Style::default().fg(TREE_GUIDE).bg(bg)),
                    ];
                    spans.push(Span::styled(name_text, Style::default().fg(name_fg).bg(bg)));
                    Cell::from(Line::from(spans))
                };

                let (state_text, state_color) = if let Some(ref s) = session {
                    let color = match s.state {
                        SessionState::Detached => GREEN,
                        SessionState::Attached => YELLOW,
                    };
                    (s.state.as_str().to_string(), color)
                } else {
                    (String::new(), DIM)
                };

                let date_text = app
                    .last_opened(name)
                    .unwrap_or_default();

                Row::new(vec![
                    name_cell,
                    Cell::from(Span::styled(
                        state_text,
                        Style::default().fg(state_color).bg(bg),
                    )),
                    Cell::from(Span::styled(
                        date_text,
                        Style::default().fg(DIM).bg(bg),
                    )),
                ])
                .style(Style::default().fg(FG).bg(bg))
            }
            ListItem::SessionItem(session) => {
                let is_current = app.is_current_session(session);

                let state_color = match session.state {
                    SessionState::Detached => GREEN,
                    SessionState::Attached => YELLOW,
                };

                let bg = if selectable_row_idx % 2 == 1 { ZEBRA_BG } else { BASE_BG };
                selectable_row_idx += 1;
                let name_fg = if is_current { ACCENT } else { GREEN };
                let prefix = if is_current { "\u{25c6} " } else { "  " };
                let avail = name_chars.saturating_sub(prefix.chars().count());
                let name_text = truncate(&session.name, avail);

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

                Row::new(vec![
                    name_cell,
                    Cell::from(Span::styled(
                        session.state.as_str().to_string(),
                        Style::default().fg(state_color).bg(bg),
                    )),
                    Cell::from(Span::styled(
                        app.last_opened(&session.name).unwrap_or_default(),
                        Style::default().fg(DIM).bg(bg),
                    )),
                ])
                .style(Style::default().fg(FG).bg(bg))
            }
        })
        .collect();

    let widths = [
        Constraint::Min(name_w),
        Constraint::Length(STATE_W),
        Constraint::Length(DATE_W),
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
        .title(
            Line::from(Span::styled(
                " Legend (?) ",
                Style::default().fg(HELP_FG).bg(BASE_BG),
            ))
            .right_aligned(),
        );

    // Bottom border: status message and/or filter indicator
    let mut bottom_spans: Vec<Span> = Vec::new();
    if app.filter_opened {
        bottom_spans.push(Span::styled(
            " Showing: opened only ",
            Style::default().fg(MATCH_FG).bg(BASE_BG),
        ));
    }
    if !app.status_msg.is_empty() {
        let is_error = app.status_msg.starts_with("Error");
        let fg = if is_error { STATUS_ERR } else { STATUS_OK };
        if !bottom_spans.is_empty() {
            bottom_spans.push(Span::styled(" ", Style::default().bg(BASE_BG)));
        }
        bottom_spans.push(Span::styled(
            format!(" {} ", app.status_msg),
            Style::default().fg(fg).bg(BASE_BG),
        ));
    }
    if !bottom_spans.is_empty() {
        block = block.title_bottom(Line::from(bottom_spans));
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
        let table = Table::new(empty_rows, widths)
            .header(header)
            .block(block)
            .style(Style::default().fg(FG).bg(BASE_BG))
            .column_spacing(COL_SPACING);
        f.render_widget(table, area);
    } else {
        let table = Table::new(rows, widths)
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

        let mut state = TableState::default();
        state.select(selected_visual_row);

        f.render_stateful_widget(table, area, &mut state);

        // Scrollbar
        let visible_rows = area.height.saturating_sub(4) as usize; // borders + header + header margin
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
    let line = Line::from(vec![
        Span::styled(
            " /",
            Style::default()
                .fg(MATCH_FG)
                .bg(SEARCH_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.search_input.clone(),
            Style::default().fg(FG_BRIGHT).bg(SEARCH_BG),
        ),
        Span::styled(
            cursor.to_string(),
            Style::default().fg(MATCH_FG).bg(SEARCH_BG),
        ),
        Span::styled(
            format!("  ({} matches)", app.selectable_indices.len()),
            Style::default().fg(COUNT_FG).bg(SEARCH_BG),
        ),
    ]);

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
            " y/Enter: confirm  n/Esc: cancel",
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

// ── Legend ───────────────────────────────────────────────────

fn draw_legend(f: &mut Frame, app: &App) {
    let in_screen = app.current_session.is_some();

    let mut entries: Vec<(&str, &str)> = vec![
        ("Enter", "Attach"),
        ("c", "Create"),
        ("n", "Rename"),
        ("x", "Kill"),
        ("X", "Kill all"),
        ("p", "Pin/Unpin"),
        ("/", "Search"),
        ("o", "Toggle filter"),
        ("r", "Refresh"),
        ("j/k", "Navigate"),
        ("g/G", "Top/Bottom"),
        ("?", "Close legend"),
        ("q", "Quit"),
    ];
    if in_screen {
        entries.insert(3, ("d", "Go home"));
    }

    let width: u16 = 22;
    let height = entries.len() as u16 + 3;
    let area = f.area();
    let x = area.width.saturating_sub(width + 2);
    let y = area.height.saturating_sub(height + 2);
    let legend_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, legend_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODAL_BORDER).bg(MODAL_BG))
        .style(Style::default().fg(FG).bg(MODAL_BG))
        .title(Span::styled(
            " Keys ",
            Style::default()
                .fg(MODAL_TITLE)
                .bg(MODAL_BG)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(legend_area);
    f.render_widget(block, legend_area);

    let lines: Vec<Line> = entries
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(
                    format!(" {key:>5}"),
                    Style::default().fg(ACCENT).bg(MODAL_BG),
                ),
                Span::styled(
                    format!("  {desc}"),
                    Style::default().fg(MODAL_TITLE).bg(MODAL_BG),
                ),
            ])
        })
        .collect();

    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FG).bg(MODAL_BG)),
        inner,
    );
}

// ── Sidebar layout ──────────────────────────────────────────

fn draw_sidebar_layout(f: &mut Frame, app: &App) {
    let area = f.area();

    // Paint entire screen with base bg
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

    let sw = app.sidebar_width(area.width);
    let sidebar_area = Rect::new(area.x, area.y, sw, area.height);
    let content_area = Rect::new(area.x + sw, area.y, area.width.saturating_sub(sw), area.height);

    // Draw sidebar (session list)
    draw_sidebar(f, app, sidebar_area);

    // Draw content area
    if app.mode == Mode::Attached {
        draw_attached_in_area(f, app, content_area);
    } else {
        draw_sidebar_empty_content(f, app, content_area);
    }

    // Draw modals on top
    match app.mode {
        Mode::Creating => {
            dim_background(f);
            draw_create_modal(f, app);
        }
        Mode::Renaming => {
            dim_background(f);
            draw_rename_modal(f, app);
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
        _ => {}
    }

    if app.show_legend {
        draw_legend(f, app);
    }
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let sidebar_focused = app.sidebar_focus == SidebarFocus::List || app.mode != Mode::Attached;
    let border_color = if sidebar_focused { ACTIVE_PANE_BORDER } else { BORDER_FG };

    let show_search_bar = app.mode == Mode::Searching || !app.search_input.is_empty();

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color).bg(BASE_BG))
        .style(Style::default().fg(FG).bg(BASE_BG))
        .title(Line::from(vec![
            Span::styled(
                " scrn ",
                Style::default()
                    .fg(ACCENT)
                    .bg(BASE_BG)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

    // Bottom border: status message
    let mut bottom_spans: Vec<Span> = Vec::new();
    if !app.status_msg.is_empty() {
        let is_error = app.status_msg.starts_with("Error");
        let fg = if is_error { STATUS_ERR } else { STATUS_OK };
        bottom_spans.push(Span::styled(
            format!(" {} ", app.status_msg),
            Style::default().fg(fg).bg(BASE_BG),
        ));
    }
    if !bottom_spans.is_empty() {
        block = block.title_bottom(Line::from(bottom_spans));
    }

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split inner area: list + optional search bar
    let (list_area, search_area) = if show_search_bar {
        let chunks = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
        ]).split(inner);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner, None)
    };

    // Draw compact session list
    let selected_visual_row = app
        .selectable_indices
        .get(app.selected)
        .copied();

    let max_name_w = list_area.width as usize;

    let rows: Vec<Row> = app
        .display_items
        .iter()
        .map(|item| match item {
            ListItem::SectionHeader(title) => {
                Row::new(vec![Cell::from(Line::from(Span::styled(
                    truncate(title, max_name_w),
                    Style::default()
                        .fg(SECTION_FG)
                        .bg(BASE_BG)
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                )))])
                .style(Style::default().fg(SECTION_FG).bg(BASE_BG))
            }
            ListItem::TreeDir { name, .. } => {
                Row::new(vec![Cell::from(Line::from(Span::styled(
                    format!("{name}/"),
                    Style::default()
                        .fg(SECTION_FG)
                        .bg(BASE_BG)
                        .add_modifier(Modifier::BOLD),
                )))])
                .style(Style::default().fg(SECTION_FG).bg(BASE_BG))
            }
            ListItem::TreeRepo { name, session, .. } => {
                let has_session = session.is_some();
                let name_fg = if has_session { GREEN } else { REPO_FG };
                let avail = max_name_w.saturating_sub(1);
                let name_text = truncate(name, avail);

                Row::new(vec![Cell::from(Line::from(vec![
                    Span::styled(" ", Style::default().fg(FG).bg(BASE_BG)),
                    Span::styled(name_text, Style::default().fg(name_fg).bg(BASE_BG)),
                ]))])
                .style(Style::default().fg(FG).bg(BASE_BG))
            }
            ListItem::SessionItem(session) => {
                let is_current = app.is_current_session(session);
                let name_fg = if is_current { ACCENT } else { GREEN };
                let indicator = if is_current { "\u{25c6}" } else { " " };
                let avail = max_name_w.saturating_sub(2);
                let name_text = truncate(&session.name, avail);

                Row::new(vec![Cell::from(Line::from(vec![
                    Span::styled(
                        format!("{indicator} "),
                        Style::default().fg(if is_current { ACCENT } else { FG }).bg(BASE_BG),
                    ),
                    Span::styled(
                        name_text,
                        Style::default().fg(name_fg).bg(BASE_BG),
                    ),
                ]))])
                .style(Style::default().fg(FG).bg(BASE_BG))
            }
        })
        .collect();

    if app.selectable_indices.is_empty() {
        let msg = if !app.search_input.is_empty() { "No matches" } else { "No sessions" };
        let empty_rows = vec![Row::new(vec![Cell::from(Span::styled(
            msg,
            Style::default().fg(DIM).bg(BASE_BG),
        ))])];
        let table = Table::new(empty_rows, [Constraint::Min(0)])
            .style(Style::default().fg(FG).bg(BASE_BG));
        f.render_widget(table, list_area);
    } else {
        let table = Table::new(rows, [Constraint::Min(0)])
            .style(Style::default().fg(FG).bg(BASE_BG))
            .row_highlight_style(
                Style::default()
                    .bg(HIGHLIGHT_BG)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("\u{2588} ");

        let mut state = TableState::default();
        state.select(selected_visual_row);
        f.render_stateful_widget(table, list_area, &mut state);
        app.sidebar_table_offset.set(state.offset());
    }

    // Draw search bar inside sidebar
    if let Some(search_area) = search_area {
        let cursor = if app.mode == Mode::Searching { "\u{2502}" } else { "" };
        let line = Line::from(vec![
            Span::styled("/", Style::default().fg(MATCH_FG).bg(SEARCH_BG).add_modifier(Modifier::BOLD)),
            Span::styled(app.search_input.clone(), Style::default().fg(FG_BRIGHT).bg(SEARCH_BG)),
            Span::styled(cursor.to_string(), Style::default().fg(MATCH_FG).bg(SEARCH_BG)),
        ]);
        f.render_widget(
            Paragraph::new(line).style(Style::default().fg(FG).bg(SEARCH_BG)),
            search_area,
        );
    }
}

fn draw_sidebar_empty_content(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_FG).bg(BASE_BG))
        .style(Style::default().fg(FG).bg(BASE_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Center a hint message
    let msg = "Select a session and press Enter";
    let msg_w = msg.len() as u16;
    if inner.width >= msg_w && inner.height >= 1 {
        let x = inner.x + (inner.width.saturating_sub(msg_w)) / 2;
        let y = inner.y + inner.height / 2;
        let hint_area = Rect::new(x, y, msg_w, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(DIM).bg(BASE_BG),
            ))),
            hint_area,
        );
    }
}

fn draw_attached_in_area(f: &mut Frame, app: &App, area: Rect) {
    let content_focused = app.sidebar_focus == SidebarFocus::Content;
    let is_two_pane = app.pty_right.is_some();

    // In two-pane mode, start with dim border; active pane highlighting is applied after.
    // In single-pane mode, highlight the whole border when content is focused.
    let border_color = if is_two_pane {
        BORDER_FG
    } else if content_focused {
        ACTIVE_PANE_BORDER
    } else {
        BORDER_FG
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color).bg(BASE_BG))
        .style(Style::default().fg(FG).bg(BASE_BG))
        .title(Line::from(vec![
            Span::styled(
                format!(" {} ", app.attached_name),
                Style::default().fg(FG_BRIGHT).bg(BASE_BG),
            ),
        ]));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if is_two_pane {
        let total_w = inner.width;
        let left_w = (total_w.saturating_sub(1)) * app.split_left_pct as u16 / 100;
        let right_w = total_w.saturating_sub(1).saturating_sub(left_w);

        let left_area = Rect::new(inner.x, inner.y, left_w, inner.height);
        let sep_x = inner.x + left_w;
        let right_area = Rect::new(sep_x + 1, inner.y, right_w, inner.height);

        // Draw vertical separator
        let buf = f.buffer_mut();
        for y in inner.top()..inner.bottom() {
            if let Some(cell) = buf.cell_mut((sep_x, y)) {
                cell.set_fg(SEPARATOR_FG);
                cell.set_bg(BASE_BG);
                cell.set_symbol("\u{2502}");
            }
        }

        // Highlight active pane border
        let c = ACTIVE_PANE_BORDER;
        let bx = area.x;
        let by = area.y;
        let bw = area.width;
        let bh = area.height;

        match app.active_pane {
            Pane::Left => {
                // Top edge: left corner to separator
                for x in bx..sep_x {
                    if let Some(cell) = buf.cell_mut((x, by)) { cell.set_fg(c); }
                }
                // Bottom edge
                for x in bx..sep_x {
                    if let Some(cell) = buf.cell_mut((x, by + bh - 1)) { cell.set_fg(c); }
                }
                // Left side
                for y in by..by + bh {
                    if let Some(cell) = buf.cell_mut((bx, y)) { cell.set_fg(c); }
                }
                // Separator
                for y in inner.top()..inner.bottom() {
                    if let Some(cell) = buf.cell_mut((sep_x, y)) { cell.set_fg(c); }
                }
                // Junction corners
                if let Some(cell) = buf.cell_mut((sep_x, by)) {
                    cell.set_symbol("\u{2510}"); // ┐
                    cell.set_fg(c);
                }
                if let Some(cell) = buf.cell_mut((sep_x, by + bh - 1)) {
                    cell.set_symbol("\u{2518}"); // ┘
                    cell.set_fg(c);
                }
            }
            Pane::Right => {
                // Top edge: after separator to right corner
                for x in (sep_x + 1)..bx + bw {
                    if let Some(cell) = buf.cell_mut((x, by)) { cell.set_fg(c); }
                }
                // Bottom edge
                for x in (sep_x + 1)..bx + bw {
                    if let Some(cell) = buf.cell_mut((x, by + bh - 1)) { cell.set_fg(c); }
                }
                // Right side
                for y in by..by + bh {
                    if let Some(cell) = buf.cell_mut((bx + bw - 1, y)) { cell.set_fg(c); }
                }
                // Separator
                for y in inner.top()..inner.bottom() {
                    if let Some(cell) = buf.cell_mut((sep_x, y)) { cell.set_fg(c); }
                }
                // Junction corners
                if let Some(cell) = buf.cell_mut((sep_x, by)) {
                    cell.set_symbol("\u{250c}"); // ┌
                    cell.set_fg(c);
                }
                if let Some(cell) = buf.cell_mut((sep_x, by + bh - 1)) {
                    cell.set_symbol("\u{2514}"); // └
                    cell.set_fg(c);
                }
            }
        }

        // Mark left pane cells as skip
        for y in left_area.top()..left_area.bottom() {
            for x in left_area.left()..left_area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.skip = true;
                }
            }
        }

        // Mark right pane cells as skip
        for y in right_area.top()..right_area.bottom() {
            for x in right_area.left()..right_area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.skip = true;
                }
            }
        }
    } else {
        // Single pane: mark inner area cells as skip
        let buf = f.buffer_mut();
        for y in inner.top()..inner.bottom() {
            for x in inner.left()..inner.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.skip = true;
                }
            }
        }
    }
}

/// Compute sidebar content area geometry for PTY rendering.
/// Returns (inner_x, inner_y, inner_w, inner_h) — the area inside the content box border.
pub fn sidebar_geometry(app: &App, cols: u16, rows: u16) -> (u16, u16, u16, u16) {
    let sw = app.sidebar_width(cols);
    let content_x = sw;
    let content_w = cols.saturating_sub(sw);
    // Content box has 1-cell border on all sides
    let inner_x = content_x + 1;
    let inner_y = 1u16;
    let inner_w = content_w.saturating_sub(2);
    let inner_h = rows.saturating_sub(2);
    (inner_x, inner_y, inner_w, inner_h)
}

/// Compute two-pane geometry within sidebar content area.
/// Returns (left_x, left_w, right_x, right_w, inner_y, inner_h).
pub fn sidebar_two_pane_geometry(app: &App, cols: u16, rows: u16) -> (u16, u16, u16, u16, u16, u16) {
    let (inner_x, inner_y, inner_w, inner_h) = sidebar_geometry(app, cols, rows);
    let left_w = (inner_w.saturating_sub(1)) * app.split_left_pct as u16 / 100;
    let right_w = inner_w.saturating_sub(1).saturating_sub(left_w);
    let left_x = inner_x;
    let right_x = inner_x + left_w + 1;
    (left_x, left_w, right_x, right_w, inner_y, inner_h)
}

// ── Attached PTY view ───────────────────────────────────────

fn draw_attached(f: &mut Frame, app: &App) {
    let area = f.area();
    let box_area = area;

    // Fill everything with base bg first
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

    let is_two_pane = app.pty_right.is_some();

    // Draw the bordered box
    let border_fg = if is_two_pane { BORDER_FG } else { BORDER_FG };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_fg).bg(BASE_BG))
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
                "> ",
                Style::default().fg(DIM).bg(BASE_BG),
            ),
            Span::styled(
                format!("{} ", app.attached_name),
                Style::default().fg(FG_BRIGHT).bg(BASE_BG),
            ),
        ]));

    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    if is_two_pane {
        let total_w = inner.width;
        let left_w = (total_w.saturating_sub(1)) * app.split_left_pct as u16 / 100;
        let right_w = total_w.saturating_sub(1).saturating_sub(left_w);

        let left_area = Rect::new(inner.x, inner.y, left_w, inner.height);
        let sep_x = inner.x + left_w;
        let right_area = Rect::new(sep_x + 1, inner.y, right_w, inner.height);

        // Draw vertical separator
        let buf = f.buffer_mut();
        for y in inner.top()..inner.bottom() {
            if let Some(cell) = buf.cell_mut((sep_x, y)) {
                cell.set_fg(SEPARATOR_FG);
                cell.set_bg(BASE_BG);
                cell.set_symbol("\u{2502}"); // │
            }
        }

        // Highlight active pane border
        let c = ACTIVE_PANE_BORDER;
        let bx = box_area.x;
        let by = box_area.y;
        let bw = box_area.width;
        let bh = box_area.height;

        match app.active_pane {
            Pane::Left => {
                // Top edge: left corner to separator (exclusive)
                for x in bx..sep_x {
                    if let Some(cell) = buf.cell_mut((x, by)) { cell.set_fg(c); }
                }
                // Bottom edge: left corner to separator (exclusive)
                for x in bx..sep_x {
                    if let Some(cell) = buf.cell_mut((x, by + bh - 1)) { cell.set_fg(c); }
                }
                // Left side
                for y in by..by + bh {
                    if let Some(cell) = buf.cell_mut((bx, y)) { cell.set_fg(c); }
                }
                // Separator inner rows
                for y in inner.top()..inner.bottom() {
                    if let Some(cell) = buf.cell_mut((sep_x, y)) { cell.set_fg(c); }
                }
                // Junction corners: ┐ top, ┘ bottom (closes the left pane box)
                if let Some(cell) = buf.cell_mut((sep_x, by)) {
                    cell.set_symbol("\u{2510}"); // ┐
                    cell.set_fg(c);
                }
                if let Some(cell) = buf.cell_mut((sep_x, by + bh - 1)) {
                    cell.set_symbol("\u{2518}"); // ┘
                    cell.set_fg(c);
                }
            }
            Pane::Right => {
                // Top edge: after separator to right corner
                for x in (sep_x + 1)..bx + bw {
                    if let Some(cell) = buf.cell_mut((x, by)) { cell.set_fg(c); }
                }
                // Bottom edge: after separator to right corner
                for x in (sep_x + 1)..bx + bw {
                    if let Some(cell) = buf.cell_mut((x, by + bh - 1)) { cell.set_fg(c); }
                }
                // Right side
                for y in by..by + bh {
                    if let Some(cell) = buf.cell_mut((bx + bw - 1, y)) { cell.set_fg(c); }
                }
                // Separator inner rows
                for y in inner.top()..inner.bottom() {
                    if let Some(cell) = buf.cell_mut((sep_x, y)) { cell.set_fg(c); }
                }
                // Junction corners: ┌ top, └ bottom (closes the right pane box)
                if let Some(cell) = buf.cell_mut((sep_x, by)) {
                    cell.set_symbol("\u{250c}"); // ┌
                    cell.set_fg(c);
                }
                if let Some(cell) = buf.cell_mut((sep_x, by + bh - 1)) {
                    cell.set_symbol("\u{2514}"); // └
                    cell.set_fg(c);
                }
            }
        }

        // Mark left pane cells as skip
        let buf = f.buffer_mut();
        for y in left_area.top()..left_area.bottom() {
            for x in left_area.left()..left_area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.skip = true;
                }
            }
        }

        // Mark right pane cells as skip
        for y in right_area.top()..right_area.bottom() {
            for x in right_area.left()..right_area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.skip = true;
                }
            }
        }
    } else {
        // Single pane: mark inner area cells as skip
        let buf = f.buffer_mut();
        for y in inner.top()..inner.bottom() {
            for x in inner.left()..inner.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.skip = true;
                }
            }
        }
    }

}

/// Compute two-pane layout geometry for render_pty_direct calls.
/// Returns (left_x, left_w, right_x, right_w, inner_y, inner_h)
pub fn two_pane_geometry(app: &App, cols: u16, rows: u16) -> (u16, u16, u16, u16, u16, u16) {
    let inner_x = 1u16;
    let inner_y = 1u16;
    let inner_w = cols.saturating_sub(2);
    let inner_h = rows.saturating_sub(2);

    let left_w = (inner_w.saturating_sub(1)) * app.split_left_pct as u16 / 100;
    let right_w = inner_w.saturating_sub(1).saturating_sub(left_w);
    let left_x = inner_x;
    let right_x = inner_x + left_w + 1; // +1 for separator

    (left_x, left_w, right_x, right_w, inner_y, inner_h)
}

/// Push the decimal representation of a u16 directly into a byte buffer,
/// avoiding `write!`/`std::fmt` overhead.
#[inline(always)]
fn push_u16(buf: &mut Vec<u8>, n: u16) {
    if n >= 10000 {
        buf.push(b'0' + (n / 10000) as u8);
    }
    if n >= 1000 {
        buf.push(b'0' + ((n / 1000) % 10) as u8);
    }
    if n >= 100 {
        buf.push(b'0' + ((n / 100) % 10) as u8);
    }
    if n >= 10 {
        buf.push(b'0' + ((n / 10) % 10) as u8);
    }
    buf.push(b'0' + (n % 10) as u8);
}

/// Push a u8 decimal representation into a byte buffer.
#[inline(always)]
fn push_u8(buf: &mut Vec<u8>, n: u8) {
    if n >= 100 {
        buf.push(b'0' + n / 100);
    }
    if n >= 10 {
        buf.push(b'0' + (n / 10) % 10);
    }
    buf.push(b'0' + n % 10);
}

/// Push CSI sequence start (`\x1b[`) into a byte buffer.
#[inline(always)]
fn push_csi(buf: &mut Vec<u8>) {
    buf.push(0x1b);
    buf.push(b'[');
}

/// Push a CUP (cursor position) sequence: `\x1b[{row};{col}H`
#[inline(always)]
fn push_cup(buf: &mut Vec<u8>, row: u16, col: u16) {
    push_csi(buf);
    push_u16(buf, row);
    buf.push(b';');
    push_u16(buf, col);
    buf.push(b'H');
}

/// Render PTY cell content directly to the terminal, bypassing ratatui.
///
/// This function ONLY writes cell content — no cursor hide/show.
/// Render plain-text history lines (from Screen's `hardcopy -h` dump).
/// Used when the user scrolls past the vt100 parser's scrollback buffer.
/// `offset` is the line index into `lines` for the top of the viewport.
pub fn render_history_lines(
    w: &mut impl Write,
    lines: &[String],
    offset: usize,
    inner_x: u16,
    inner_y: u16,
    inner_w: u16,
    inner_h: u16,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(inner_w as usize * inner_h as usize * 4);
    let default_style = CellStyle::default_cell();
    write_sgr(&mut buf, &default_style);

    for row in 0..inner_h {
        push_cup(&mut buf, inner_y + row + 1, inner_x + 1);
        let line_idx = offset + row as usize;
        let line = lines.get(line_idx).map(|s| s.as_str()).unwrap_or("");
        let mut col = 0u16;
        for ch in line.chars() {
            if col >= inner_w {
                break;
            }
            let mut tmp = [0u8; 4];
            let s = ch.encode_utf8(&mut tmp);
            buf.extend_from_slice(s.as_bytes());
            col += 1;
        }
        // Fill remaining columns with spaces
        while col < inner_w {
            buf.push(b' ');
            col += 1;
        }
    }

    if !buf.is_empty() {
        buf.extend_from_slice(b"\x1b[0m");
        w.write_all(&buf)?;
    }
    Ok(())
}

/// Cursor visibility is managed by the caller at the frame level to
/// avoid flicker from repeated hide/show cycles.
///
/// When `prev_screen` is provided, rows where every cell matches the
/// previous frame are skipped entirely — zero bytes emitted for those rows.
pub fn render_pty_direct(
    w: &mut impl Write,
    screen: &vt100::Screen,
    prev_screen: Option<&vt100::Screen>,
    inner_x: u16,
    inner_y: u16,
    inner_w: u16,
    inner_h: u16,
) -> std::io::Result<()> {
    let (scr_rows, scr_cols) = screen.size();
    let mut buf = Vec::with_capacity(inner_w as usize * inner_h as usize * 8);

    for row in 0..inner_h.min(scr_rows) {
        // Differential: skip unchanged rows
        if let Some(prev) = prev_screen {
            let (prev_rows, prev_cols) = prev.size();
            if prev_rows == scr_rows && prev_cols == scr_cols {
                let cols_to_check = inner_w.min(scr_cols);
                let mut row_changed = false;
                for col in 0..cols_to_check {
                    let cur = screen.cell(row, col);
                    let old = prev.cell(row, col);
                    if cur != old {
                        row_changed = true;
                        break;
                    }
                }
                if !row_changed {
                    continue;
                }
            }
        }

        push_cup(&mut buf, inner_y + row + 1, inner_x + 1);

        // Reset prev_style per rendered row so SGR state is correct after skipping
        let mut prev_style: Option<CellStyle> = None;

        for col in 0..inner_w.min(scr_cols) {
            if let Some(vt_cell) = screen.cell(row, col) {
                if vt_cell.is_wide_continuation() {
                    continue;
                }

                let style = CellStyle::from_vt(vt_cell);
                if prev_style.as_ref() != Some(&style) {
                    write_sgr(&mut buf, &style);
                    prev_style = Some(style);
                }

                let contents = vt_cell.contents();
                if contents.is_empty() {
                    buf.push(b' ');
                } else {
                    buf.extend_from_slice(contents.as_bytes());
                }
            }
        }

        let filled = inner_w.min(scr_cols);
        if filled < inner_w {
            let fill_style = CellStyle::default_cell();
            if prev_style.as_ref() != Some(&fill_style) {
                write_sgr(&mut buf, &fill_style);
            }
            for _ in filled..inner_w {
                buf.push(b' ');
            }
        }
    }

    let filled_rows = inner_h.min(scr_rows);
    if filled_rows < inner_h {
        // Only emit blank fill rows if there's no prev_screen or size changed
        let need_fill = prev_screen.map_or(true, |prev| {
            let (pr, _) = prev.size();
            pr != scr_rows
        });
        if need_fill {
            let fill_style = CellStyle::default_cell();
            write_sgr(&mut buf, &fill_style);
            for row in filled_rows..inner_h {
                push_cup(&mut buf, inner_y + row + 1, inner_x + 1);
                for _ in 0..inner_w {
                    buf.push(b' ');
                }
            }
        }
    }

    if !buf.is_empty() {
        buf.extend_from_slice(b"\x1b[0m");
        w.write_all(&buf)?;
    }
    Ok(())
}

/// Position the terminal cursor at the PTY app's cursor location and show it,
/// but only if the PTY app itself wants the cursor visible.
///
/// Call this ONCE per frame, after all render_pty_direct calls, for the active pane.
pub fn write_pty_cursor(
    w: &mut impl Write,
    screen: &vt100::Screen,
    inner_x: u16,
    inner_y: u16,
) -> std::io::Result<()> {
    if screen.hide_cursor() {
        return Ok(());
    }
    let (cr, cc) = screen.cursor_position();
    let cursor_x = inner_x + cc + 1;
    let cursor_y = inner_y + cr + 1;
    let mut buf = Vec::with_capacity(16);
    push_cup(&mut buf, cursor_y, cursor_x);
    buf.extend_from_slice(b"\x1b[?25h");
    w.write_all(&buf)
}

#[derive(PartialEq, Clone)]
pub struct CellStyle {
    fg: vt100::Color,
    bg: vt100::Color,
    bold: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
}

impl CellStyle {
    fn from_vt(cell: &vt100::Cell) -> Self {
        Self {
            fg: cell.fgcolor(),
            bg: cell.bgcolor(),
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }

    fn default_cell() -> Self {
        Self {
            fg: vt100::Color::Default,
            bg: vt100::Color::Default,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

fn write_sgr(buf: &mut Vec<u8>, s: &CellStyle) {
    buf.extend_from_slice(b"\x1b[0");
    if s.bold {
        buf.extend_from_slice(b";1");
    }
    if s.italic {
        buf.extend_from_slice(b";3");
    }
    if s.underline {
        buf.extend_from_slice(b";4");
    }
    if s.inverse {
        write_color(buf, s.bg, true);
        write_color(buf, s.fg, false);
    } else {
        write_color(buf, s.fg, true);
        write_color(buf, s.bg, false);
    }
    buf.push(b'm');
}

fn write_color(buf: &mut Vec<u8>, color: vt100::Color, is_fg: bool) {
    match color {
        vt100::Color::Default => {
            if is_fg {
                buf.extend_from_slice(b";38;2;220;220;230");
            } else {
                buf.extend_from_slice(b";48;2;18;18;24");
            }
        }
        vt100::Color::Idx(i) if i < 8 => {
            buf.push(b';');
            push_u8(buf, if is_fg { 30 + i } else { 40 + i });
        }
        vt100::Color::Idx(i) if i < 16 => {
            buf.push(b';');
            push_u8(buf, if is_fg { 90 + (i - 8) } else { 100 + (i - 8) });
        }
        vt100::Color::Idx(i) => {
            buf.push(b';');
            push_u8(buf, if is_fg { 38 } else { 48 });
            buf.extend_from_slice(b";5;");
            push_u8(buf, i);
        }
        vt100::Color::Rgb(r, g, b) => {
            buf.push(b';');
            push_u8(buf, if is_fg { 38 } else { 48 });
            buf.extend_from_slice(b";2;");
            push_u8(buf, r);
            buf.push(b';');
            push_u8(buf, g);
            buf.push(b';');
            push_u8(buf, b);
        }
    }
}

/// Render a scrollbar on the right edge of the pane when scrolled back.
/// The scrollbar overlays the rightmost column of the content area.
pub fn render_scrollbar(
    w: &mut impl Write,
    scroll_offset: usize,
    total_scrollback: usize,
    inner_x: u16,
    inner_y: u16,
    inner_w: u16,
    inner_h: u16,
) -> std::io::Result<()> {
    if total_scrollback == 0 || inner_h < 2 || inner_w < 2 {
        return Ok(());
    }

    let track_height = inner_h as usize;
    let total_content = total_scrollback + inner_h as usize;

    // Thumb size: proportional to viewport vs total content, at least 1
    let thumb_size = ((track_height * track_height) / total_content).max(1);

    // Position: scroll_offset=0 → thumb at bottom (live), scroll_offset=max → thumb at top
    let max_scroll = total_scrollback;
    let scroll_fraction = if max_scroll > 0 {
        scroll_offset as f64 / max_scroll as f64
    } else {
        0.0
    };

    let max_thumb_pos = track_height.saturating_sub(thumb_size);
    let thumb_top = ((1.0 - scroll_fraction) * max_thumb_pos as f64) as usize;

    let x = inner_x + inner_w; // rightmost column (overlays border)
    let mut buf = Vec::with_capacity(track_height * 16);

    for i in 0..track_height {
        push_cup(&mut buf, inner_y + i as u16 + 1, x);
        if i >= thumb_top && i < thumb_top + thumb_size {
            // Thumb: bright block
            buf.extend_from_slice(b"\x1b[0;38;2;140;140;200m\xe2\x96\x88"); // █
        } else {
            // Track: dim line
            buf.extend_from_slice(b"\x1b[0;38;2;40;40;60m\xe2\x94\x82"); // │
        }
    }
    buf.extend_from_slice(b"\x1b[0m");

    // Line position indicator at top-right of content area
    let mut label = Vec::with_capacity(32);
    label.extend_from_slice(b" [");
    push_u16(&mut label, scroll_offset as u16);
    label.push(b'/');
    push_u16(&mut label, total_scrollback as u16);
    label.extend_from_slice(b"] ");

    let len = label.len() as u16;
    if len < inner_w {
        let lx = inner_x + inner_w - len;
        push_cup(&mut buf, inner_y + 1, lx + 1);
        buf.extend_from_slice(b"\x1b[0;7m"); // reverse video
        buf.extend_from_slice(&label);
        buf.extend_from_slice(b"\x1b[0m");
    }

    w.write_all(&buf)
}

// ── Text selection ──────────────────────────────────────────

#[derive(Clone)]
pub struct PaneSelection {
    pub start_row: u16,
    pub start_col: u16,
    pub end_row: u16,
    pub end_col: u16,
}

impl PaneSelection {
    /// Normalize so that start <= end (for iteration).
    pub fn normalized(&self) -> (u16, u16, u16, u16) {
        if self.start_row < self.end_row
            || (self.start_row == self.end_row && self.start_col <= self.end_col)
        {
            (self.start_row, self.start_col, self.end_row, self.end_col)
        } else {
            (self.end_row, self.end_col, self.start_row, self.start_col)
        }
    }

    pub fn is_non_empty(&self) -> bool {
        self.start_row != self.end_row || self.start_col != self.end_col
    }
}

/// Overlay selected cells with a fixed highlight background.
pub fn render_selection(
    w: &mut impl Write,
    screen: &vt100::Screen,
    sel: &PaneSelection,
    inner_x: u16,
    inner_y: u16,
    inner_w: u16,
    inner_h: u16,
) -> std::io::Result<()> {
    let (sr, sc, er, ec) = sel.normalized();
    let (scr_rows, scr_cols) = screen.size();
    let max_row = er.min(inner_h.saturating_sub(1)).min(scr_rows.saturating_sub(1));
    let mut buf = Vec::with_capacity(256);

    for row in sr..=max_row {
        let col_start = if row == sr { sc } else { 0 };
        let col_end = if row == er { ec } else { inner_w.saturating_sub(1) };
        let col_end = col_end.min(inner_w.saturating_sub(1)).min(scr_cols.saturating_sub(1));
        if col_start > col_end {
            continue;
        }

        push_cup(&mut buf, inner_y + row + 1, inner_x + col_start + 1);
        let mut prev_style: Option<CellStyle> = None;

        for col in col_start..=col_end {
            if let Some(vt_cell) = screen.cell(row, col) {
                if vt_cell.is_wide_continuation() {
                    continue;
                }
                let mut style = CellStyle::from_vt(vt_cell);
                // Fixed selection background (steel blue) — keeps original fg visible
                style.bg = vt100::Color::Rgb(60, 70, 100);
                style.inverse = false;
                if prev_style.as_ref() != Some(&style) {
                    write_sgr(&mut buf, &style);
                    prev_style = Some(style);
                }
                let contents = vt_cell.contents();
                if contents.is_empty() {
                    buf.push(b' ');
                } else {
                    buf.extend_from_slice(contents.as_bytes());
                }
            }
        }
    }

    if !buf.is_empty() {
        buf.extend_from_slice(b"\x1b[0m");
        w.write_all(&buf)?;
    }
    Ok(())
}

/// Extract text content from the selected region of a PTY screen.
pub fn extract_selection_text(
    screen: &vt100::Screen,
    sel: &PaneSelection,
    inner_w: u16,
    inner_h: u16,
) -> String {
    let (sr, sc, er, ec) = sel.normalized();
    let (scr_rows, scr_cols) = screen.size();
    let max_row = er.min(inner_h.saturating_sub(1)).min(scr_rows.saturating_sub(1));
    let mut result = String::new();

    for row in sr..=max_row {
        let col_start = if row == sr { sc } else { 0 };
        let col_end = if row == er { ec } else { inner_w.saturating_sub(1) };
        let col_end = col_end.min(inner_w.saturating_sub(1)).min(scr_cols.saturating_sub(1));

        let mut line = String::new();
        for col in col_start..=col_end {
            if let Some(cell) = screen.cell(row, col) {
                let c = cell.contents();
                if c.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&c);
                }
            }
        }
        if row < max_row {
            result.push_str(line.trim_end());
            result.push('\n');
        } else {
            result.push_str(line.trim_end());
        }
    }
    result
}
