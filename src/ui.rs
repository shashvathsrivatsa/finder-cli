use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, Paragraph},
};

use crate::app::{App, ClipboardOp, CLIPBOARD_FLASH_MS};

pub fn render(frame: &mut Frame, app: &mut App) {
    let full_area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(full_area);
    let area = chunks[0];
    let status_area = chunks[1];

    if let Some(path) = &app.confirming_delete {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let kind = if path.is_dir() { "directory" } else { "file" };
        let text = Line::from(vec![
            Span::styled(format!("Delete {} ", kind), Style::default().fg(Color::Rgb(220, 50, 50))),
            Span::styled(format!("\"{}\"", name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("?  ", Style::default().fg(Color::Rgb(220, 50, 50))),
            Span::styled("[y]", Style::default().fg(Color::Rgb(80, 200, 120)).add_modifier(Modifier::BOLD)),
            Span::styled("es  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[n]", Style::default().fg(Color::Rgb(220, 50, 50)).add_modifier(Modifier::BOLD)),
            Span::styled("o", Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(text), status_area);
    } else if let Some((_, dst)) = &app.confirming_replace {
        let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let text = Line::from(vec![
            Span::styled("Replace ", Style::default().fg(Color::Rgb(220, 140, 50))),
            Span::styled(format!("\"{}\"", name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("?  ", Style::default().fg(Color::Rgb(220, 140, 50))),
            Span::styled("[y]", Style::default().fg(Color::Rgb(80, 200, 120)).add_modifier(Modifier::BOLD)),
            Span::styled("es  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[n]", Style::default().fg(Color::Rgb(220, 50, 50)).add_modifier(Modifier::BOLD)),
            Span::styled("o", Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(text), status_area);
    } else if let Some(cb) = &app.clipboard {
        if cb.set_at.elapsed().as_millis() < CLIPBOARD_FLASH_MS as u128 {
            let name = cb.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (verb, color) = match cb.op {
                ClipboardOp::Cut  => ("cut",  Color::Rgb(220, 140, 50)),
                ClipboardOp::Copy => ("copy", Color::Rgb(100, 180, 255)),
            };
            let text = Line::from(vec![
                Span::styled(format!("{}: ", verb), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(Color::DarkGray)),
            ]);
            frame.render_widget(Paragraph::new(text), status_area);
        }
    }

    let num_cols = app.columns.len();

    const COL_WIDTH: u16 = 32;
    let visible_cols = ((area.width / COL_WIDTH) as usize).max(1).min(num_cols);
    let preferred_start = app.active_col.saturating_sub(visible_cols.saturating_sub(2));
    let start_col = preferred_start.min(num_cols.saturating_sub(visible_cols));

    let visible_count = (num_cols - start_col).min(visible_cols);
    let mut constraints: Vec<Constraint> =
        (0..visible_count).map(|_| Constraint::Length(COL_WIDTH)).collect();
    constraints.push(Constraint::Min(0));

    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (vi, ci) in (start_col..num_cols).enumerate() {
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
}
