//! TUI for selecting the split point in `gg unstack`.

use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::error::{GgError, Result};

/// A commit entry for the unstack TUI.
#[derive(Debug, Clone)]
pub struct UnstackEntry {
    /// Current stack position.
    pub position: usize,
    /// Short SHA for display.
    pub short_sha: String,
    /// Stable GG-ID, when present.
    pub gg_id: Option<String>,
    /// Commit title.
    pub title: String,
}

struct UnstackTuiState {
    entries: Vec<UnstackEntry>,
    cursor: usize,
    confirmed: bool,
    aborted: bool,
}

impl UnstackTuiState {
    fn new(entries: Vec<UnstackEntry>) -> Self {
        Self {
            entries,
            cursor: 1,
            confirmed: false,
            aborted: false,
        }
    }

    fn cursor_up(&mut self) {
        if self.cursor > 1 {
            self.cursor -= 1;
        }
    }

    fn cursor_down(&mut self) {
        if self.cursor + 1 < self.entries.len() {
            self.cursor += 1;
        }
    }

    fn selected_position(&self) -> usize {
        self.cursor + 1
    }
}

struct TerminalGuard {
    _private: (),
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()
            .map_err(|e| GgError::Other(format!("Failed to enable raw mode: {}", e)))?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(GgError::Other(format!(
                "Failed to enter alternate screen: {}",
                e
            )));
        }
        Ok(Self { _private: () })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Run the unstack target picker.
///
/// Returns the 1-indexed position where the new stack should begin.
pub fn unstack_tui(entries: Vec<UnstackEntry>) -> Result<Option<usize>> {
    if entries.len() < 2 {
        return Err(GgError::Other(
            "Need at least 2 commits to unstack".to_string(),
        ));
    }

    let _guard = TerminalGuard::new()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|e| GgError::Other(format!("Failed to create terminal: {}", e)))?;
    let mut state = UnstackTuiState::new(entries);

    loop {
        terminal
            .draw(|f| draw(f, &state))
            .map_err(|e| GgError::Other(format!("Failed to draw: {}", e)))?;

        if event::poll(std::time::Duration::from_millis(100))
            .map_err(|e| GgError::Other(format!("Event poll failed: {}", e)))?
        {
            if let Event::Key(key) =
                event::read().map_err(|e| GgError::Other(format!("Event read failed: {}", e)))?
            {
                handle_key(&mut state, key.code, key.modifiers);

                if state.confirmed {
                    return Ok(Some(state.selected_position()));
                }
                if state.aborted {
                    return Ok(None);
                }
            }
        }
    }
}

fn handle_key(state: &mut UnstackTuiState, code: KeyCode, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('c')) {
        state.aborted = true;
        return;
    }

    match code {
        KeyCode::Up | KeyCode::Char('k') => state.cursor_up(),
        KeyCode::Down | KeyCode::Char('j') => state.cursor_down(),
        KeyCode::Enter | KeyCode::Char('s') => state.confirmed = true,
        KeyCode::Char('q') | KeyCode::Esc => state.aborted = true,
        _ => {}
    }
}

fn draw(f: &mut Frame, state: &UnstackTuiState) {
    let size = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(size);

    draw_commit_list(f, state, chunks[0]);
    draw_status_bar(f, chunks[1]);
}

fn draw_commit_list(f: &mut Frame, state: &UnstackTuiState, area: ratatui::layout::Rect) {
    let split_position = state.selected_position();
    let items: Vec<ListItem> = state
        .entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let is_current = idx == state.cursor;
            let is_invalid = idx == 0;
            let cursor_marker = if is_current { ">" } else { " " };
            let boundary_marker = if entry.position == split_position {
                "NEW "
            } else if entry.position > split_position {
                "    "
            } else {
                "OLD "
            };

            let mut spans = vec![
                Span::raw(cursor_marker),
                Span::raw(" "),
                Span::styled(
                    boundary_marker,
                    if entry.position >= split_position {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled(
                    format!("{:<3}", entry.position),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{} ", entry.short_sha),
                    Style::default().fg(Color::Yellow),
                ),
            ];

            if let Some(gg_id) = &entry.gg_id {
                spans.push(Span::styled(
                    format!("{} ", gg_id),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            spans.push(Span::raw(&entry.title));

            if is_invalid {
                spans.push(Span::styled(
                    "  cannot start at first commit",
                    Style::default().fg(Color::Red),
                ));
            }

            let mut item = ListItem::new(Line::from(spans));
            if is_current {
                item = item.style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
            }
            item
        })
        .collect();

    let block = Block::default()
        .title(" Unstack: select new stack start ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    f.render_widget(List::new(items).block(block), area);
}

fn draw_status_bar(f: &mut Frame, area: ratatui::layout::Rect) {
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::Gray);
    let sep_style = Style::default().fg(Color::DarkGray);

    let line = Line::from(vec![
        Span::styled(" j", key_style),
        Span::styled("/", sep_style),
        Span::styled("k", key_style),
        Span::styled(" select", desc_style),
        Span::styled("  ", sep_style),
        Span::styled("Enter", key_style),
        Span::styled("/", sep_style),
        Span::styled("s", key_style),
        Span::styled(" confirm", desc_style),
        Span::styled("  ", sep_style),
        Span::styled("q", key_style),
        Span::styled("/", sep_style),
        Span::styled("Esc", key_style),
        Span::styled(" cancel", desc_style),
    ]);

    f.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries(n: usize) -> Vec<UnstackEntry> {
        (1..=n)
            .map(|i| UnstackEntry {
                position: i,
                short_sha: format!("abc{:04}", i),
                gg_id: Some(format!("c-{:07x}", i)),
                title: format!("commit {}", i),
            })
            .collect()
    }

    #[test]
    fn state_starts_on_second_commit() {
        let state = UnstackTuiState::new(make_entries(3));
        assert_eq!(state.cursor, 1);
        assert_eq!(state.selected_position(), 2);
    }

    #[test]
    fn cursor_never_selects_first_commit() {
        let mut state = UnstackTuiState::new(make_entries(3));
        state.cursor_up();
        assert_eq!(state.selected_position(), 2);
    }
}
