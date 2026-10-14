//! A guard's question, in a terminal.
//!
//! The navigator parks a move and holds the question as plain data - three
//! strings. Drawing them is all a backend has to do, and a terminal can do it
//! as well as any window manager: a box in the middle, and the two keys named
//! on the buttons the others would draw.

use guinea_core::guard::Ask;
use guinea_router::navigator::Asking;
use ratatui::Frame;
use ratatui::crossterm::event::{Event, KeyCode};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// Y confirms, N or Esc cancels. Named on screen rather than assumed, because
/// a terminal has no button to read them off.
pub(crate) fn answer(asking: &dyn Asking, event: &Event) {
    let Some(code) = crate::keys::pressed(event) else {
        return;
    };

    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => asking.answer(true),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => asking.answer(false),
        _ => {}
    }
}

pub(crate) fn draw(frame: &mut Frame, ask: &Ask) {
    let area = centred(frame.area(), 56, 7);

    // `Clear` first: the page underneath drew here, and a box that lets it
    // through is unreadable.
    frame.render_widget(Clear, area);

    let choices = Line::from(vec![
        Span::styled("[y] ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(ask.confirm.clone()),
        Span::raw("   "),
        Span::styled("[n] ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(ask.cancel.clone()),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ? ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(ask.text.clone()).wrap(Wrap { trim: true }),
        rows[0],
    );
    frame.render_widget(Paragraph::new(choices).alignment(Alignment::Center), rows[1]);
}

fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);

    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
