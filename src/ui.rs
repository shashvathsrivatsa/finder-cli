use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};

use crate::app::{App, ClipboardOp, PaneInfo, CLIPBOARD_FLASH_MS};

pub fn render(frame: &mut Frame, app: &mut App) {
    let full_area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(full_area);
    let area = chunks[0];
    let status_area = chunks[1];

    // Always show link icon on the left if a pane is linked
    let link_prefix: Vec<Span> = if app.linked_pane.is_some() {
        vec![Span::styled(" \u{F0C1} ", Style::default().fg(Color::Rgb(100, 180, 255)))]
    } else {
        vec![]
    };

    let status_spans: Option<Vec<Span>> = if let Some(path) = &app.confirming_delete {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let kind = if path.is_dir() { "directory" } else { "file" };
        Some(vec![
            Span::styled(format!("Delete {} ", kind), Style::default().fg(Color::Rgb(220, 50, 50))),
            Span::styled(format!("\"{}\"", name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("?  ", Style::default().fg(Color::Rgb(220, 50, 50))),
            Span::styled("[y]", Style::default().fg(Color::Rgb(80, 200, 120)).add_modifier(Modifier::BOLD)),
            Span::styled("es  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[n]", Style::default().fg(Color::Rgb(220, 50, 50)).add_modifier(Modifier::BOLD)),
            Span::styled("o", Style::default().fg(Color::DarkGray)),
        ])
    } else if let Some(cb) = &app.clipboard {
        if cb.set_at.elapsed().as_millis() < CLIPBOARD_FLASH_MS as u128 {
            let name = cb.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (verb, color) = match cb.op {
                ClipboardOp::Cut  => ("cut",  Color::Rgb(220, 140, 50)),
                ClipboardOp::Copy => ("copy", Color::Rgb(100, 180, 255)),
            };
            Some(vec![
                Span::styled(format!("{}: ", verb), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(Color::DarkGray)),
            ])
        } else { None }
    } else { None };

    if !link_prefix.is_empty() || status_spans.is_some() {
        let mut spans = link_prefix;
        if let Some(s) = status_spans { spans.extend(s); }
        frame.render_widget(Paragraph::new(Line::from(spans)), status_area);
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

        let block = Block::bordered()
            .title(Span::styled(
                format!(" {} ", folder_name),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(Color::Rgb(60, 60, 60)))
            .style(Style::default().bg(Color::Black));

        let inner = block.inner(col_chunks[vi]);
        frame.render_widget(block, col_chunks[vi]);

        let selected_path = col.selected_entry().map(|e| e.path.clone());
        // Only pass rename input for the active column
        let renaming = if is_active { app.renaming.as_ref() } else { None };
        let (items, _) = col.grouped.list_items(selected_path.as_deref(), renaming);

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
