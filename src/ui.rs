use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};

use chrono::{DateTime, Local, Datelike, Timelike};

use crate::app::{App, ClipboardOp, ConvertState, PaneInfo, PreviewMode, CLIPBOARD_FLASH_MS, YtdlpState};

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
    } else if app.is_downloading {
        let pct = app.ytdlp_progress.as_ref().map(|p| p.load(std::sync::atomic::Ordering::Relaxed));
        let label = match pct {
            Some(p) if p != u64::MAX => format!("Downloading... {}%", p),
            _ => "Downloading...".to_string(),
        };
        Some(vec![
            Span::styled(format!("{} ", spinner_ch), Style::default().fg(Color::Rgb(255, 100, 180))),
            Span::styled(label, Style::default().fg(Color::Rgb(255, 100, 180)).add_modifier(Modifier::BOLD)),
        ])
    } else if app.is_converting {
        Some(vec![
            Span::styled(format!("{} ", spinner_ch), Style::default().fg(Color::Rgb(180, 140, 255))),
            Span::styled("Converting...", Style::default().fg(Color::Rgb(180, 140, 255)).add_modifier(Modifier::BOLD)),
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
    } else if let Some((ref msg, ref at)) = app.status_flash {
        if at.elapsed().as_secs() < 3 {
            Some(vec![Span::styled(msg.clone(), Style::default().fg(Color::Rgb(220, 50, 50)).add_modifier(Modifier::BOLD))])
        } else {
            app.status_flash = None;
            None
        }
    } else { None };

    // Status takes priority — when active, hide preview entirely.
    // Name mode expands the bar height; short/long stay at 1 line.
    let showing_status = !link_prefix.is_empty() || status_spans.is_some() || app.converting.is_some() || app.ytdlp.is_some();

    let status_height: u16 = if app.ytdlp.is_some() {
        match &app.ytdlp {
            Some(YtdlpState::FormatPicker { formats, .. }) => ytdlp_format_height(formats.len(), full_area.width as usize) as u16,
            _ => 1,
        }
    } else if app.converting.is_some() {
        app.converting.as_ref().map(|cs| convert_bar_height(cs, full_area.width as usize)).unwrap_or(1) as u16
    } else if !showing_status && app.preview_mode == PreviewMode::Name {
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

    if let Some(ref yt) = app.ytdlp {
        match yt {
            YtdlpState::UrlInput(q) => {
                let spans = vec![
                    Span::styled("yt-dlp  ", Style::default().fg(Color::Rgb(255, 100, 180)).add_modifier(Modifier::BOLD)),
                    Span::styled(q.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("█", Style::default().fg(Color::Rgb(255, 100, 180))),
                ];
                frame.render_widget(Paragraph::new(Line::from(spans)), status_area);
            }
            YtdlpState::FetchingFormats(_) => {
                let spans = vec![
                    Span::styled(format!("{} ", spinner_ch), Style::default().fg(Color::Rgb(255, 100, 180))),
                    Span::styled("Fetching formats...", Style::default().fg(Color::Rgb(255, 100, 180)).add_modifier(Modifier::BOLD)),
                ];
                frame.render_widget(Paragraph::new(Line::from(spans)), status_area);
            }
            YtdlpState::FormatPicker { formats, selected, .. } => {
                let lines = ytdlp_format_lines(formats, *selected, status_area.width as usize);
                frame.render_widget(Paragraph::new(lines), status_area);
            }
        }
    } else if let Some(ref cs) = app.converting {
        let lines = convert_bar_lines(cs, status_area.width as usize);
        frame.render_widget(Paragraph::new(lines), status_area);
    } else if showing_status {
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
                    let ready_i64 = |cell: &Option<std::sync::Arc<std::sync::atomic::AtomicI64>>| -> Option<i64> {
                        cell.as_ref().and_then(|c| {
                            let v = c.load(std::sync::atomic::Ordering::Relaxed);
                            if v == i64::MIN { None } else { Some(v) }
                        })
                    };
                    let dims_str = ready_i64(&app.preview_dims).and_then(|v| {
                        if v < 0 { None } else {
                            let w = (v >> 32) as u32;
                            let h = (v & 0xFFFFFFFF) as u32;
                            Some(format!("{}×{}", w, h))
                        }
                    });
                    let fps_str = ready_i64(&app.preview_fps).and_then(|v| {
                        if v < 0 { None } else {
                            let fps = v as f64 / 100.0;
                            let s = if fps.fract() < 0.05 { format!("{}p", fps.round() as u32) }
                                    else { format!("{:.2}p", fps) };
                            Some(s)
                        }
                    });
                    let dim_fps_str = match (&dims_str, &fps_str) {
                        (Some(d), Some(f)) => Some(format!("dim: {}/{}", d, f)),
                        (Some(d), None)    => Some(format!("dim: {}", d)),
                        _                  => None,
                    };
                    let dur_str = ready_i64(&app.preview_duration).and_then(|v| {
                        if v < 0 { return None; }
                        let secs = v as u64;
                        let h = secs / 3600;
                        let m = (secs % 3600) / 60;
                        let s = secs % 60;
                        let formatted = if h > 0 { format!("{}h {}m", h, m) }
                            else if m > 0 { format!("{}m {}s", m, s) }
                            else { format!("{:.1}s", v as f64) };
                        Some(format!("dur: {}", formatted))
                    });
                    let pages_str = ready_i64(&app.preview_pages).and_then(|v| {
                        if v > 0 { Some(format!("pages: {}", v)) } else { None }
                    });
                    let parts: Vec<String> = if app.preview_mode == PreviewMode::Long {
                        [size_str, modified_str, created_str, count_str, dim_fps_str, dur_str, pages_str].into_iter().flatten().collect()
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

fn convert_bar_lines(cs: &ConvertState, width: usize) -> Vec<Line<'static>> {
    let name = cs.source.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let prefix_str = format!("convert  {}  →  ", name);
    let prefix_len = prefix_str.chars().count();
    // measure each option: " fmt " + "  " padding = fmt.len() + 4
    let option_widths: Vec<usize> = cs.formats.iter().map(|f| f.len() + 4).collect();
    // pack options into lines
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut remaining = width.saturating_sub(prefix_len);
    let mut current_spans: Vec<Span<'static>> = vec![
        Span::styled("convert  ", Style::default().fg(Color::Rgb(180, 140, 255)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{}  →  ", name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ];
    let indent = " ".repeat(prefix_len);
    for (i, &fmt) in cs.formats.iter().enumerate() {
        let w = option_widths[i];
        if current_spans.len() > 2 && w > remaining {
            lines.push(Line::from(std::mem::replace(&mut current_spans,
                vec![Span::raw(indent.clone())])));
            remaining = width.saturating_sub(prefix_len);
        }
        if i == cs.selected {
            current_spans.push(Span::styled(
                format!(" {} ", fmt),
                Style::default().fg(Color::Black).bg(Color::Rgb(180, 140, 255)).add_modifier(Modifier::BOLD),
            ));
        } else {
            current_spans.push(Span::styled(
                format!(" {} ", fmt),
                Style::default().fg(Color::Rgb(140, 100, 200)),
            ));
        }
        current_spans.push(Span::raw("  "));
        remaining = remaining.saturating_sub(w);
    }
    if !current_spans.is_empty() { lines.push(Line::from(current_spans)); }
    lines
}

fn convert_bar_height(cs: &ConvertState, width: usize) -> usize {
    convert_bar_lines(cs, width).len().max(1)
}

fn ytdlp_format_lines(formats: &[crate::app::DynYtFormat], selected: usize, width: usize) -> Vec<Line<'static>> {
    let prefix = "yt-dlp  quality  →  ";
    let prefix_len = prefix.chars().count();
    let indent = " ".repeat(prefix_len);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = vec![
        Span::styled("yt-dlp  ", Style::default().fg(Color::Rgb(255, 100, 180)).add_modifier(Modifier::BOLD)),
        Span::styled("quality  →  ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ];
    let mut remaining = width.saturating_sub(prefix_len);
    for (i, fmt) in formats.iter().enumerate() {
        let w = fmt.label.len() + 4;
        if current.len() > 2 && w > remaining {
            lines.push(Line::from(std::mem::replace(&mut current, vec![Span::raw(indent.clone())])));
            remaining = width.saturating_sub(prefix_len);
        }
        let label = fmt.label.clone();
        if i == selected {
            current.push(Span::styled(format!(" {} ", label),
                Style::default().fg(Color::Black).bg(Color::Rgb(255, 100, 180)).add_modifier(Modifier::BOLD)));
        } else {
            current.push(Span::styled(format!(" {} ", label),
                Style::default().fg(Color::Rgb(200, 80, 140))));
        }
        current.push(Span::raw("  "));
        remaining = remaining.saturating_sub(w);
    }
    if !current.is_empty() { lines.push(Line::from(current)); }
    lines
}

fn ytdlp_format_height(n_formats: usize, width: usize) -> usize {
    let dummy: Vec<crate::app::DynYtFormat> = (0..n_formats).map(|_| crate::app::DynYtFormat {
        label: "xxx".to_string(), format_arg: String::new(), extra_args: vec![],
    }).collect();
    ytdlp_format_lines(&dummy, 0, width).len().max(1)
}
