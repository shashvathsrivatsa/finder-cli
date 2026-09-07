use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};

use chrono::{DateTime, Local, Datelike, Timelike};

use crate::app::{App, ClipboardOp, PaneInfo, PreviewMode, CLIPBOARD_FLASH_MS};

fn format_size(bytes: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = K * 1024;
    const G: u64 = M * 1024;
    if bytes >= G      { format!("{:.1}G", bytes as f64 / G as f64) }
    else if bytes >= M { format!("{:.1}M", bytes as f64 / M as f64) }
    else if bytes >= K { format!("{:.1}K", bytes as f64 / K as f64) }
    else               { format!("{}B", bytes) }
}

fn format_ts(secs: i64) -> Option<String> {
    let dt: DateTime<Local> = DateTime::from_timestamp(secs, 0)?.with_timezone(&Local);
    let now = Local::now();
    let ap = |pm: bool| if pm { "p" } else { "a" };
    let s = if dt.year() != now.year() {
        let (pm, h) = dt.hour12();
        format!("{}/{}/{} {}:{:02}{}", dt.month(), dt.day(), dt.year(), h, dt.minute(), ap(pm))
    } else if dt.month() == now.month() && dt.day() == now.day() {
        let (pm, h) = dt.hour12();
        format!("{}:{:02}{}", h, dt.minute(), ap(pm))
    } else {
        let (pm, h) = dt.hour12();
        format!("{}/{} {}:{:02}{}", dt.month(), dt.day(), h, dt.minute(), ap(pm))
    };
    Some(s)
}

pub fn render(frame: &mut Frame, app: &mut App) {
    let full_area = frame.area();

    // Build left status spans first so we can measure their width
    let link_prefix: Vec<Span> = if app.linked_pane.is_some() {
        vec![Span::styled(" \u{F0C1} ", Style::default().fg(Color::Rgb(100, 180, 255)))]
    } else {
        vec![]
    };

    const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let spinner_ch = SPINNER[app.spinner_frame % SPINNER.len()];

    let bg_done = app.bg_progress.as_ref()
        .map(|p| p.load(std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(0);
    let bg_total = app.bg_total.as_ref()
        .map(|t| t.load(std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(0);

    let status_spans: Option<Vec<Span>> = if app.is_deleting {
        let label = if bg_total > 0 {
            format!("Deleting ({}/{})...", bg_done, bg_total)
        } else {
            "Deleting...".to_string()
        };
        Some(vec![
            Span::styled(format!("{} ", spinner_ch), Style::default().fg(Color::Rgb(220, 50, 50))),
            Span::styled(label, Style::default().fg(Color::Rgb(220, 50, 50)).add_modifier(Modifier::BOLD)),
        ])
    } else if app.is_pasting {
        let label = if bg_total > 0 {
            format!("Pasting ({}/{})...", bg_done, bg_total)
        } else {
            "Pasting...".to_string()
        };
        Some(vec![
            Span::styled(format!("{} ", spinner_ch), Style::default().fg(Color::Rgb(100, 180, 255))),
            Span::styled(label, Style::default().fg(Color::Rgb(100, 180, 255)).add_modifier(Modifier::BOLD)),
        ])
    } else if let Some(ref q) = app.goto_query {
        Some(vec![
            Span::styled("/", Style::default().fg(Color::Rgb(255, 200, 80)).add_modifier(Modifier::BOLD)),
            Span::styled(q.clone(), Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Rgb(255, 200, 80))),
        ])
    } else if let Some(path) = &app.confirming_delete {
        let multi = app.pending_deletes.len() > 1;
        let (label, name) = if multi {
            ("Delete ".to_string(), format!("{} items", app.pending_deletes.len()))
        } else {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let kind = if path.is_dir() { "directory" } else { "file" };
            (format!("Delete {} ", kind), format!("\"{}\"", name))
        };
        Some(vec![
            Span::styled(label, Style::default().fg(Color::Rgb(220, 50, 50))),
            Span::styled(name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("?  ", Style::default().fg(Color::Rgb(220, 50, 50))),
            Span::styled("[y]", Style::default().fg(Color::Rgb(80, 200, 120)).add_modifier(Modifier::BOLD)),
            Span::styled("es  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[n]", Style::default().fg(Color::Rgb(220, 50, 50)).add_modifier(Modifier::BOLD)),
            Span::styled("o", Style::default().fg(Color::DarkGray)),
        ])
    } else if app.select_mode {
        Some(vec![
            Span::styled("-- VISUAL --", Style::default().fg(Color::Rgb(80, 200, 120)).add_modifier(Modifier::BOLD)),
        ])
    } else if let Some(cb) = &app.clipboard {
        if cb.set_at.elapsed().as_millis() < CLIPBOARD_FLASH_MS as u128 {
            let name = cb.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (verb, color) = match cb.op {
                ClipboardOp::Cut  => ("move", Color::Rgb(220, 140, 50)),
                ClipboardOp::Copy => ("copy", Color::Rgb(100, 180, 255)),
            };
            Some(vec![
                Span::styled(format!("{}: ", verb), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(Color::DarkGray)),
            ])
        } else { None }
    } else { None };

    // Status takes priority — when active, hide preview entirely.
    // Name mode expands the bar height; short/long stay at 1 line.
    let showing_status = !link_prefix.is_empty() || status_spans.is_some();

    let status_height: u16 = if !showing_status && app.preview_mode == PreviewMode::Name {
        let col = &app.columns[app.active_col];
        let name = col.grouped.entry_at_row(col.selected_row)
            .map(|e| e.name.as_str())
            .unwrap_or("");
        let w = full_area.width.max(1) as usize;
        let max_lines = (full_area.height / 2).max(1);
        let lines_needed = ((name.len() + w - 1) / w).max(1) as u16;
        lines_needed.min(max_lines)
    } else { 1 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(status_height)])
        .split(full_area);
    let area = chunks[0];
    let status_area = chunks[1];

    if showing_status {
        let mut spans = link_prefix;
        if let Some(s) = status_spans { spans.extend(s); }
        frame.render_widget(Paragraph::new(Line::from(spans)), status_area);
    } else {
        // No active status — show preview
        let preview_spans: Option<Vec<Span>> = match app.preview_mode {
            PreviewMode::Name => {
                let col = &app.columns[app.active_col];
                let name = col.grouped.entry_at_row(col.selected_row)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                let w = status_area.width.max(1) as usize;
                let max_chars = w * status_area.height as usize;
                let display = if name.len() <= max_chars {
                    name
                } else {
                    let mut s = name[..max_chars.saturating_sub(3)].to_string();
                    s.push_str("...");
                    s
                };
                let lines: Vec<Line> = display.as_bytes().chunks(w)
                    .map(|c| Line::from(Span::styled(
                        String::from_utf8_lossy(c).into_owned(),
                        Style::default().fg(Color::DarkGray),
                    )))
                    .collect();
                frame.render_widget(Paragraph::new(lines), status_area);
                None
            }
            PreviewMode::Short | PreviewMode::Long => {
                {
                    let ready = |cell: &Option<std::sync::Arc<std::sync::atomic::AtomicI64>>| -> Option<i64> {
                        cell.as_ref().and_then(|c| {
                            let v = c.load(std::sync::atomic::Ordering::Relaxed);
                            if v == i64::MIN { None } else { Some(v) }
                        })
                    };
                    let size_str = app.preview_size.as_ref().and_then(|cell| {
                        let v = cell.load(std::sync::atomic::Ordering::Relaxed);
                        if v == u64::MAX { None } else { Some(format!("size: {}", format_size(v))) }
                    });
                    let modified_str = ready(&app.preview_modified)
                        .and_then(|s| if s < 0 { None } else { format_ts(s) })
                        .map(|s| format!("modified: {}", s));
                    let created_str = ready(&app.preview_created)
                        .and_then(|s| if s < 0 { None } else { format_ts(s) })
                        .map(|s| format!("created: {}", s));
                    let count_str = ready(&app.preview_count).and_then(|c| {
                        if c >= 0 { Some(format!("items: {}", c)) } else { None }
                    });
                    let parts: Vec<String> = if app.preview_mode == PreviewMode::Long {
                        [size_str, modified_str, created_str, count_str].into_iter().flatten().collect()
                    } else {
                        // Short: just the raw size without label
                        app.preview_size.as_ref().and_then(|cell| {
                            let v = cell.load(std::sync::atomic::Ordering::Relaxed);
                            if v == u64::MAX { None } else { Some(vec![format_size(v)]) }
                        }).unwrap_or_default()
                    };
                    let text = if parts.is_empty() {
                        format!("{} Scanning...", spinner_ch)
                    } else { parts.join("  ") };
                    Some(vec![Span::styled(text, Style::default().fg(Color::DarkGray))])
                }
            }
        };
        if let Some(spans) = preview_spans {
            frame.render_widget(Paragraph::new(Line::from(spans)), status_area);
        }
    }

    let num_cols = app.columns.len();

    const COL_WIDTH: u16 = 32;
    let fits = area.width / COL_WIDTH;
    let visible_cols = (fits as usize).max(1).min(num_cols);
    let single_pane = fits < 2; // not enough room for even two columns
    let preferred_start = app.active_col.saturating_sub(visible_cols.saturating_sub(2));
    let start_col = preferred_start.min(num_cols.saturating_sub(visible_cols));

    let visible_count = (num_cols - start_col).min(visible_cols);
    let constraints: Vec<Constraint> = if single_pane {
        // stretch the single active column to fill all available space
        vec![Constraint::Min(0)]
    } else {
        let mut c: Vec<Constraint> = (0..visible_count).map(|_| Constraint::Length(COL_WIDTH)).collect();
        c.push(Constraint::Min(0));
        c
    };

    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    let col_range: Box<dyn Iterator<Item = usize>> = if single_pane {
        Box::new(std::iter::once(app.active_col))
    } else {
        Box::new(start_col..start_col + visible_count)
    };
    for (vi, ci) in col_range.enumerate() {
        let col = &mut app.columns[ci];
        let is_active = ci == app.active_col;

        let folder_name = col
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string());

        let inner = if single_pane {
            let col_area = col_chunks[vi];
            // Render just the folder name as a top heading line
            let heading = Paragraph::new(Line::from(vec![
                Span::styled(folder_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]));
            frame.render_widget(heading, Rect { x: col_area.x + 1, y: col_area.y, width: col_area.width.saturating_sub(1), height: 1 });
            // List gets everything below the heading, inset by 1 on the left to match bordered layout
            Rect { x: col_area.x + 1, y: col_area.y + 1, width: col_area.width.saturating_sub(1), height: col_area.height.saturating_sub(1) }
        } else {
            let block = Block::bordered()
                .title(Span::styled(
                    format!(" {} ", folder_name),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(Color::Rgb(60, 60, 60)))
                .style(Style::default().bg(Color::Black));
            let inner = block.inner(col_chunks[vi]);
            frame.render_widget(block, col_chunks[vi]);
            inner
        };

        if is_active {
            app.col_viewport_height = inner.height as usize;
        }
        let selected_path = col.selected_entry().map(|e| e.path.clone());
        // Only pass rename input for the active column
        let renaming = if is_active { app.renaming.as_ref() } else { None };
        let (items, _) = col.grouped.list_items(selected_path.as_deref(), renaming, &app.selection, &app.favorites);

        let highlight_style = if is_active && app.renaming.is_some() {
            Style::default()
        } else if is_active && app.focused {
            Style::default().bg(Color::Rgb(0, 92, 197)).add_modifier(Modifier::BOLD)
        } else if is_active {
            Style::default().bg(Color::Rgb(60, 60, 60))
        } else {
            Style::default().bg(Color::Rgb(60, 60, 60))
        };

        let list = List::new(items)
            .highlight_style(highlight_style)
            .style(Style::default().fg(Color::Rgb(200, 200, 200)));

        frame.render_stateful_widget(list, inner, &mut col.list_state);
    }

    if app.favorites_view {
        let mut favs: Vec<&std::path::PathBuf> = app.favorites.iter().collect();
        favs.sort();

        let overlay_w = (area.width * 2 / 3).max(40).min(area.width);
        let overlay_h = (favs.len() as u16 + 4).min(area.height.saturating_sub(4)).max(5);
        let overlay = Rect {
            x: area.x + (area.width.saturating_sub(overlay_w)) / 2,
            y: area.y + (area.height.saturating_sub(overlay_h)) / 2,
            width: overlay_w,
            height: overlay_h,
        };
        frame.render_widget(Clear, overlay);

        let block = Block::bordered()
            .title(Span::styled(" Favorites ", Style::default().fg(Color::Rgb(255, 200, 50)).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::Rgb(255, 200, 50)))
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(overlay);
        frame.render_widget(block, overlay);

        let items: Vec<ListItem> = if favs.is_empty() {
            vec![ListItem::new(Span::styled("  No favorites yet. Press f to add.", Style::default().fg(Color::DarkGray)))]
        } else {
            favs.iter().map(|p| {
                let is_dir = p.is_dir();
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned());
                let display_name = if is_dir { format!("{}/", name) } else { name };
                let parent = p.parent().and_then(|p| p.to_str()).unwrap_or("").to_string();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {}", display_name), Style::default().fg(Color::White)),
                    Span::styled(format!("  {}", parent), Style::default().fg(Color::Rgb(180, 180, 180))),
                ]))
            }).collect()
        };

        let cursor = app.favorites_cursor.min(favs.len().saturating_sub(1));
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(if favs.is_empty() { None } else { Some(cursor) });

        let list = List::new(items)
            .highlight_style(Style::default().bg(Color::Rgb(0, 92, 197)).add_modifier(Modifier::BOLD));
        frame.render_stateful_widget(list, inner, &mut list_state);
    }

    if let Some((ref panes, sel)) = app.pane_picker {
        let current_count = panes.iter().filter(|p| p.same_session).count();
        let has_others = panes.len() > current_count;
        let has_current = current_count > 0;

        // Build items list with section headers, track pane_idx -> item_idx
        let mut items: Vec<ListItem> = Vec::new();
        let mut pane_to_item: Vec<usize> = Vec::new();

        let linked_id = app.linked_pane.as_ref().map(|lp| lp.id.as_str());

        let make_pane_item = |p: &PaneInfo| {
            let is_linked = linked_id.is_some_and(|id| id == p.id);
            if is_linked {
                ListItem::new(Line::from(vec![
                    Span::styled("  \u{F0C1}  ", Style::default().fg(Color::Rgb(100, 180, 255))),
                    Span::raw(p.label.clone()),
                ]))
            } else {
                ListItem::new(Line::from(vec![Span::raw(format!("  {}", p.label))]))
            }
        };

        if has_current {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  This session", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            ])));
            for p in panes.iter().take(current_count) {
                pane_to_item.push(items.len());
                items.push(make_pane_item(p));
            }
        }
        if has_others {
            if has_current { items.push(ListItem::new(Line::from(""))); }
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  Other", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            ])));
            for p in panes.iter().skip(current_count) {
                pane_to_item.push(items.len());
                items.push(make_pane_item(p));
            }
        }

        let visual_sel = pane_to_item.get(sel).copied().unwrap_or(0);
        let height = (items.len() as u16 + 2).min(full_area.height.saturating_sub(2));
        let width = 50u16.min(full_area.width.saturating_sub(4));
        let x = (full_area.width.saturating_sub(width)) / 2;
        let y = (full_area.height.saturating_sub(height)) / 2;
        let popup_area = Rect { x, y, width, height };

        let list = List::new(items)
            .block(Block::bordered()
                .title(Span::styled(" Pick pane  [u] unlink ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)))
                .border_style(Style::default().fg(Color::Rgb(100, 180, 255)))
                .style(Style::default().bg(Color::Black)))
            .highlight_style(Style::default().bg(Color::Rgb(0, 92, 197)).add_modifier(Modifier::BOLD));

        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(visual_sel));

        frame.render_widget(Clear, popup_area);
        frame.render_stateful_widget(list, popup_area, &mut list_state);
    }
}
