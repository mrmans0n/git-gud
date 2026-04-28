//! TUI for interactive split-point selection in `gg unstack`
//!
//! A single-panel terminal UI (ratatui + crossterm) for choosing where to
//! split a stack:
//! - Navigate with j/k or arrows
//! - Enter/s to confirm, q/Esc to cancel
//! - Entries at and above the cursor are shown as "MOVE" (new stack)
//! - Entries below the cursor are shown as "stay" (old stack)

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

/// A stack entry for display in the unstack TUI
#[derive(Debug, Clone)]
pub struct UnstackEntry {
    /// Short SHA for display
    pub short_sha: String,
    /// Commit title (first line)
    pub title: String,
}

/// State for the unstack TUI
struct UnstackTuiState {
    /// Stack entries (index 0 = position 1 = bottom of stack)
    entries: Vec<UnstackEntry>,
    /// Current cursor position (index into entries). This is the split point:
    /// entries at and above cursor move to the new stack.
    cursor: usize,
    /// Whether user confirmed the split point
    confirmed: bool,
    /// Whether user cancelled
    aborted: bool,
}

impl UnstackTuiState {
    fn new(entries: Vec<UnstackEntry>, initial: usize) -> Self {
        Self {
            cursor: initial.min(entries.len().saturating_sub(1)),
            entries,
            confirmed: false,
            aborted: false,
        }
    }

    fn cursor_up(&mut self) {
        // Can't go above index 1 (position 2) — splitting at position 1 is rejected
        if self.cursor > 1 {
            self.cursor -= 1;
        }
    }

    fn cursor_down(&mut self) {
        if self.cursor + 1 < self.entries.len() {
            self.cursor += 1;
        }
    }

    /// 1-indexed split position
    fn split_position(&self) -> usize {
        self.cursor + 1
    }
}

/// Terminal cleanup guard — restores terminal on drop
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

/// Run the unstack TUI.
///
/// Returns `Ok(Some(position))` with the 1-indexed split position if confirmed,
/// or `Ok(None)` if cancelled.
///
/// `initial` is a 0-indexed cursor starting position.
pub fn select_split_point(entries: Vec<UnstackEntry>, initial: usize) -> Result<Option<usize>> {
    if entries.len() < 2 {
        return Err(GgError::Other(
            "Need at least 2 commits to unstack".to_string(),
        ));
    }

    let _guard = TerminalGuard::new()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|e| GgError::Other(format!("Failed to create terminal: {}", e)))?;

    let mut state = UnstackTuiState::new(entries, initial);

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
                    return Ok(Some(state.split_position()));
                }
                if state.aborted {
                    return Ok(None);
                }
            }
        }
    }
}

/// Handle a key press
fn handle_key(state: &mut UnstackTuiState, code: KeyCode, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('c') = code {
            state.aborted = true;
            return;
        }
    }

    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.cursor_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.cursor_down();
        }
        KeyCode::Enter | KeyCode::Char('s') => {
            state.confirmed = true;
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            state.aborted = true;
        }
        _ => {}
    }
}

/// Draw the TUI
fn draw(f: &mut Frame, state: &UnstackTuiState) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(size);

    draw_entry_list(f, state, chunks[0]);
    draw_status_bar(f, chunks[1]);
}

/// Draw the entry list with remain/move markers
fn draw_entry_list(f: &mut Frame, state: &UnstackTuiState, area: ratatui::layout::Rect) {
    let move_count = state.entries.len() - state.cursor;
    let remain_count = state.cursor;

    let items: Vec<ListItem> = state
        .entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let position = idx + 1;
            let is_cursor = idx == state.cursor;
            let will_move = idx >= state.cursor;

            let cursor_marker = if is_cursor { "▸" } else { " " };

            let tag = if will_move { "MOVE" } else { "stay" };
            let tag_style = if will_move {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let sha_style = if will_move {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let title_style = if will_move {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let spans = vec![
                Span::raw(cursor_marker),
                Span::raw(" "),
                Span::styled(format!("[{tag:<4}] "), tag_style),
                Span::styled(
                    format!("{position:<3}"),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!("{} ", entry.short_sha), sha_style),
                Span::styled(entry.title.as_str(), title_style),
            ];

            let line = Line::from(spans);
            let mut item = ListItem::new(line);

            if is_cursor {
                item = item.style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
            }

            item
        })
        .collect();

    let title = format!(" Unstack — {remain_count} remain, {move_count} move to new stack ",);

    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

/// Draw the status bar
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
        Span::styled(" move split point", desc_style),
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

    let paragraph = Paragraph::new(line).wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries(n: usize) -> Vec<UnstackEntry> {
        (1..=n)
            .map(|i| UnstackEntry {
                short_sha: format!("abc{i:04}"),
                title: format!("commit {i}"),
            })
            .collect()
    }

    #[test]
    fn test_state_initialization() {
        let state = UnstackTuiState::new(make_entries(4), 2);
        assert_eq!(state.cursor, 2);
        assert!(!state.confirmed);
        assert!(!state.aborted);
    }

    #[test]
    fn test_initial_clamped_to_max() {
        let state = UnstackTuiState::new(make_entries(3), 10);
        assert_eq!(state.cursor, 2);
    }

    #[test]
    fn test_cursor_cannot_go_below_index_1() {
        let mut state = UnstackTuiState::new(make_entries(4), 1);
        state.cursor_up();
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn test_cursor_down_stops_at_end() {
        let mut state = UnstackTuiState::new(make_entries(3), 2);
        state.cursor_down();
        assert_eq!(state.cursor, 2);
    }

    #[test]
    fn test_cursor_navigation() {
        let mut state = UnstackTuiState::new(make_entries(5), 2);
        assert_eq!(state.cursor, 2);

        state.cursor_down();
        assert_eq!(state.cursor, 3);

        state.cursor_down();
        assert_eq!(state.cursor, 4);

        state.cursor_down();
        assert_eq!(state.cursor, 4);

        state.cursor_up();
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn test_split_position_is_1_indexed() {
        let state = UnstackTuiState::new(make_entries(4), 2);
        assert_eq!(state.split_position(), 3);
    }

    #[test]
    fn test_key_navigation() {
        let mut state = UnstackTuiState::new(make_entries(5), 2);

        handle_key(&mut state, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 3);

        handle_key(&mut state, KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 2);

        handle_key(&mut state, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(state.cursor, 3);

        handle_key(&mut state, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(state.cursor, 2);
    }

    #[test]
    fn test_key_confirm_enter() {
        let mut state = UnstackTuiState::new(make_entries(3), 1);
        assert!(!state.confirmed);
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
    }

    #[test]
    fn test_key_confirm_s() {
        let mut state = UnstackTuiState::new(make_entries(3), 1);
        handle_key(&mut state, KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(state.confirmed);
    }

    #[test]
    fn test_key_cancel_q() {
        let mut state = UnstackTuiState::new(make_entries(3), 1);
        handle_key(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(state.aborted);
    }

    #[test]
    fn test_key_cancel_esc() {
        let mut state = UnstackTuiState::new(make_entries(3), 1);
        handle_key(&mut state, KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.aborted);
    }

    #[test]
    fn test_ctrl_c_aborts() {
        let mut state = UnstackTuiState::new(make_entries(3), 1);
        handle_key(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(state.aborted);
    }
}
