//! TUI for interactive commit reordering in `gg reorder`
//!
//! A single-panel terminal UI (ratatui + crossterm) for reordering commits:
//! - Navigate with j/k or arrows
//! - Move commits with J/K or Shift+arrows
//! - Confirm with Enter/s, cancel with q/Esc

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

/// A commit entry for the reorder TUI
#[derive(Debug, Clone)]
pub struct ReorderEntry {
    /// Short SHA for display
    pub short_sha: String,
    /// Commit title (first line)
    pub title: String,
}

/// State for the reorder TUI
struct ReorderTuiState {
    /// Commits in current order (index 0 = position 1 = bottom of stack)
    entries: Vec<ReorderEntry>,
    /// Current cursor position
    cursor: usize,
    /// Whether user confirmed the new order
    confirmed: bool,
    /// Whether user cancelled
    aborted: bool,
    /// Whether the order has been modified
    modified: bool,
    /// Indices of entries marked for dropping
    dropped: std::collections::HashSet<usize>,
}

impl ReorderTuiState {
    fn new(entries: Vec<ReorderEntry>) -> Self {
        Self {
            entries,
            cursor: 0,
            confirmed: false,
            aborted: false,
            modified: false,
            dropped: std::collections::HashSet::new(),
        }
    }

    /// Move cursor up
    fn cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor down
    fn cursor_down(&mut self) {
        if self.cursor + 1 < self.entries.len() {
            self.cursor += 1;
        }
    }

    /// Move the commit at cursor position up (swap with previous)
    fn move_up(&mut self) {
        if self.cursor > 0 {
            self.entries.swap(self.cursor, self.cursor - 1);
            self.swap_drop_state(self.cursor, self.cursor - 1);
            self.cursor -= 1;
            self.modified = true;
        }
    }

    /// Move the commit at cursor position down (swap with next)
    fn move_down(&mut self) {
        if self.cursor + 1 < self.entries.len() {
            self.entries.swap(self.cursor, self.cursor + 1);
            self.swap_drop_state(self.cursor, self.cursor + 1);
            self.cursor += 1;
            self.modified = true;
        }
    }

    /// Swap drop state between two indices so drop marks follow the entries
    fn swap_drop_state(&mut self, a: usize, b: usize) {
        let a_dropped = self.dropped.contains(&a);
        let b_dropped = self.dropped.contains(&b);
        if a_dropped {
            self.dropped.remove(&a);
            self.dropped.insert(b);
        }
        if b_dropped {
            self.dropped.remove(&b);
            self.dropped.insert(a);
        }
        // If both were dropped or neither was, the swap is a no-op on the set
    }

    /// Toggle drop mark on the commit at the current cursor position.
    /// At least one commit must remain (not all can be dropped).
    fn toggle_drop(&mut self) {
        if self.dropped.contains(&self.cursor) {
            self.dropped.remove(&self.cursor);
            self.modified = true;
        } else {
            // Don't allow dropping all commits
            let kept = self.entries.len() - self.dropped.len();
            if kept > 1 {
                self.dropped.insert(self.cursor);
                self.modified = true;
            }
        }
    }

    /// Get the new order as a list of short SHAs, excluding dropped entries
    fn get_order(&self) -> Vec<String> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(idx, _)| !self.dropped.contains(idx))
            .map(|(_, e)| e.short_sha.clone())
            .collect()
    }
}

/// Terminal cleanup guard - restores terminal on drop
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

/// Run the reorder TUI.
///
/// Returns `Ok(Some(shas))` with the new order of short SHAs if confirmed,
/// or `Ok(None)` if cancelled.
///
/// Entries are ordered bottom-to-top: index 0 = position 1 (closest to base).
pub fn reorder_tui(entries: Vec<ReorderEntry>) -> Result<Option<Vec<String>>> {
    if entries.len() < 2 {
        return Err(GgError::Other(
            "Need at least 2 commits to reorder".to_string(),
        ));
    }

    let _guard = TerminalGuard::new()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|e| GgError::Other(format!("Failed to create terminal: {}", e)))?;

    let mut state = ReorderTuiState::new(entries);

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
                    return Ok(Some(state.get_order()));
                }
                if state.aborted {
                    return Ok(None);
                }
            }
        }
    }
}

/// Handle a key press
fn handle_key(state: &mut ReorderTuiState, code: KeyCode, modifiers: KeyModifiers) {
    // Ctrl+C always aborts
    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('c') = code {
            state.aborted = true;
            return;
        }
    }

    match code {
        // Navigation
        KeyCode::Up | KeyCode::Char('k') if !modifiers.contains(KeyModifiers::SHIFT) => {
            state.cursor_up();
        }
        KeyCode::Down | KeyCode::Char('j') if !modifiers.contains(KeyModifiers::SHIFT) => {
            state.cursor_down();
        }

        // Move commit (Shift+arrow or J/K)
        KeyCode::Up if modifiers.contains(KeyModifiers::SHIFT) => {
            state.move_up();
        }
        KeyCode::Down if modifiers.contains(KeyModifiers::SHIFT) => {
            state.move_down();
        }
        KeyCode::Char('K') => {
            state.move_up();
        }
        KeyCode::Char('J') => {
            state.move_down();
        }

        // Confirm
        KeyCode::Enter | KeyCode::Char('s') => {
            state.confirmed = true;
        }

        // Drop/undrop
        KeyCode::Char('d') | KeyCode::Delete => {
            state.toggle_drop();
        }

        // Cancel
        KeyCode::Char('q') | KeyCode::Esc => {
            state.aborted = true;
        }

        _ => {}
    }
}

/// Draw the TUI
fn draw(f: &mut Frame, state: &ReorderTuiState) {
    let size = f.area();

    // Layout: commit list + status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(size);

    draw_commit_list(f, state, chunks[0]);
    draw_status_bar(f, state, chunks[1]);
}

/// Draw the commit list
fn draw_commit_list(f: &mut Frame, state: &ReorderTuiState, area: ratatui::layout::Rect) {
    let items: Vec<ListItem> = state
        .entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let position = idx + 1;
            let is_current = idx == state.cursor;
            let is_dropped = state.dropped.contains(&idx);

            let cursor_marker = if is_current { "▸" } else { " " };

            // Build the line
            let mut spans = if is_dropped {
                let drop_style = Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::CROSSED_OUT);
                vec![
                    Span::raw(cursor_marker),
                    Span::raw(" "),
                    Span::styled("[DROP] ", Style::default().fg(Color::Red)),
                    Span::styled(format!("{:<3}", position), drop_style),
                    Span::styled(format!("{} ", entry.short_sha), drop_style),
                    Span::styled(entry.title.as_str(), drop_style),
                ]
            } else {
                vec![
                    Span::raw(cursor_marker),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:<3}", position),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("{} ", entry.short_sha),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(&entry.title),
                ]
            };

            // Add move indicator for the selected commit
            if is_current {
                spans.push(Span::styled("  ↕", Style::default().fg(Color::Cyan)));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);

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

    let drop_count = state.dropped.len();
    let title = if drop_count > 0 {
        format!(" Arrange Stack (modified, {} to drop) ", drop_count)
    } else if state.modified {
        " Arrange Stack (modified) ".to_string()
    } else {
        " Arrange Stack ".to_string()
    };

    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

/// Draw the status bar
fn draw_status_bar(f: &mut Frame, _state: &ReorderTuiState, area: ratatui::layout::Rect) {
    let status_text =
        " j/k:navigate  J/K:move commit  d:drop/undrop  Enter/s:confirm  q/Esc:cancel ";

    let paragraph = Paragraph::new(status_text)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries(n: usize) -> Vec<ReorderEntry> {
        (1..=n)
            .map(|i| ReorderEntry {
                short_sha: format!("abc{:04}", i),
                title: format!("commit {}", i),
            })
            .collect()
    }

    #[test]
    fn test_state_initialization() {
        let entries = make_entries(3);
        let state = ReorderTuiState::new(entries.clone());

        assert_eq!(state.entries.len(), 3);
        assert_eq!(state.cursor, 0);
        assert!(!state.confirmed);
        assert!(!state.aborted);
        assert!(!state.modified);
    }

    #[test]
    fn test_cursor_up_down() {
        let mut state = ReorderTuiState::new(make_entries(4));

        assert_eq!(state.cursor, 0);

        state.cursor_down();
        assert_eq!(state.cursor, 1);

        state.cursor_down();
        assert_eq!(state.cursor, 2);

        state.cursor_down();
        assert_eq!(state.cursor, 3);

        // Can't go past end
        state.cursor_down();
        assert_eq!(state.cursor, 3);

        state.cursor_up();
        assert_eq!(state.cursor, 2);

        state.cursor_up();
        assert_eq!(state.cursor, 1);

        state.cursor_up();
        assert_eq!(state.cursor, 0);

        // Can't go before start
        state.cursor_up();
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_move_up() {
        let mut state = ReorderTuiState::new(make_entries(3));
        // entries: [1, 2, 3], cursor at 0

        // Move at top does nothing
        state.move_up();
        assert_eq!(state.cursor, 0);
        assert_eq!(state.entries[0].short_sha, "abc0001");
        assert!(!state.modified);

        // Move cursor to position 1, then move up
        state.cursor_down(); // cursor=1
        state.move_up(); // swap [1] and [0], cursor=0
        assert_eq!(state.cursor, 0);
        assert_eq!(state.entries[0].short_sha, "abc0002");
        assert_eq!(state.entries[1].short_sha, "abc0001");
        assert!(state.modified);
    }

    #[test]
    fn test_move_down() {
        let mut state = ReorderTuiState::new(make_entries(3));
        // entries: [1, 2, 3], cursor at 0

        state.move_down(); // swap [0] and [1], cursor=1
        assert_eq!(state.cursor, 1);
        assert_eq!(state.entries[0].short_sha, "abc0002");
        assert_eq!(state.entries[1].short_sha, "abc0001");
        assert!(state.modified);

        // Move at bottom does nothing
        state.cursor_down(); // cursor=2
        let prev_entries: Vec<String> = state.entries.iter().map(|e| e.short_sha.clone()).collect();
        state.move_down();
        assert_eq!(state.cursor, 2);
        let curr_entries: Vec<String> = state.entries.iter().map(|e| e.short_sha.clone()).collect();
        assert_eq!(prev_entries, curr_entries);
    }

    #[test]
    fn test_move_preserves_cursor_follows_commit() {
        let mut state = ReorderTuiState::new(make_entries(4));
        // entries: [1, 2, 3, 4]

        // Move commit 1 (at index 0) all the way down
        state.move_down(); // [2, 1, 3, 4], cursor=1
        state.move_down(); // [2, 3, 1, 4], cursor=2
        state.move_down(); // [2, 3, 4, 1], cursor=3

        assert_eq!(state.entries[0].short_sha, "abc0002");
        assert_eq!(state.entries[1].short_sha, "abc0003");
        assert_eq!(state.entries[2].short_sha, "abc0004");
        assert_eq!(state.entries[3].short_sha, "abc0001");
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn test_get_order() {
        let mut state = ReorderTuiState::new(make_entries(3));
        // Original order
        assert_eq!(state.get_order(), vec!["abc0001", "abc0002", "abc0003"]);

        // Swap first two
        state.move_down();
        assert_eq!(state.get_order(), vec!["abc0002", "abc0001", "abc0003"]);
    }

    #[test]
    fn test_key_navigation_j_k() {
        let mut state = ReorderTuiState::new(make_entries(3));

        handle_key(&mut state, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 1);

        handle_key(&mut state, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 2);

        handle_key(&mut state, KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 1);

        handle_key(&mut state, KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_key_navigation_arrows() {
        let mut state = ReorderTuiState::new(make_entries(3));

        handle_key(&mut state, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(state.cursor, 1);

        handle_key(&mut state, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_key_move_j_k_uppercase() {
        let mut state = ReorderTuiState::new(make_entries(3));
        // entries: [1, 2, 3]

        handle_key(&mut state, KeyCode::Char('J'), KeyModifiers::SHIFT);
        // Moved entry 1 down: [2, 1, 3], cursor=1
        assert_eq!(state.cursor, 1);
        assert_eq!(state.entries[0].short_sha, "abc0002");
        assert_eq!(state.entries[1].short_sha, "abc0001");

        handle_key(&mut state, KeyCode::Char('K'), KeyModifiers::SHIFT);
        // Moved entry 1 back up: [1, 2, 3], cursor=0
        assert_eq!(state.cursor, 0);
        assert_eq!(state.entries[0].short_sha, "abc0001");
    }

    #[test]
    fn test_key_move_shift_arrows() {
        let mut state = ReorderTuiState::new(make_entries(3));

        handle_key(&mut state, KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(state.cursor, 1);
        assert_eq!(state.entries[0].short_sha, "abc0002");
        assert_eq!(state.entries[1].short_sha, "abc0001");

        handle_key(&mut state, KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(state.cursor, 0);
        assert_eq!(state.entries[0].short_sha, "abc0001");
    }

    #[test]
    fn test_key_confirm_enter() {
        let mut state = ReorderTuiState::new(make_entries(2));

        assert!(!state.confirmed);
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
    }

    #[test]
    fn test_key_confirm_s() {
        let mut state = ReorderTuiState::new(make_entries(2));

        assert!(!state.confirmed);
        handle_key(&mut state, KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(state.confirmed);
    }

    #[test]
    fn test_key_cancel_q() {
        let mut state = ReorderTuiState::new(make_entries(2));

        assert!(!state.aborted);
        handle_key(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(state.aborted);
    }

    #[test]
    fn test_key_cancel_esc() {
        let mut state = ReorderTuiState::new(make_entries(2));

        assert!(!state.aborted);
        handle_key(&mut state, KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.aborted);
    }

    #[test]
    fn test_ctrl_c_aborts() {
        let mut state = ReorderTuiState::new(make_entries(2));

        assert!(!state.aborted);
        handle_key(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(state.aborted);
    }

    #[test]
    fn test_ctrl_c_plain_c_does_not_abort() {
        let mut state = ReorderTuiState::new(make_entries(2));

        handle_key(&mut state, KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(!state.aborted);
    }

    #[test]
    fn test_positions_update_after_reorder() {
        let mut state = ReorderTuiState::new(make_entries(3));
        // entries: [1, 2, 3]

        // Move commit at position 0 down twice
        state.move_down(); // [2, 1, 3], cursor=1
        state.move_down(); // [2, 3, 1], cursor=2

        let order = state.get_order();
        assert_eq!(order, vec!["abc0002", "abc0003", "abc0001"]);

        // Verify entries are in the expected order
        assert_eq!(state.entries[0].title, "commit 2");
        assert_eq!(state.entries[1].title, "commit 3");
        assert_eq!(state.entries[2].title, "commit 1");
    }

    #[test]
    fn test_two_entries_minimum() {
        let entries = make_entries(2);
        let state = ReorderTuiState::new(entries);
        assert_eq!(state.entries.len(), 2);
    }

    #[test]
    fn test_single_swap_round_trip() {
        let mut state = ReorderTuiState::new(make_entries(2));
        // [1, 2]
        state.move_down(); // [2, 1], cursor=1
        state.move_up(); // [1, 2], cursor=0
        assert_eq!(state.get_order(), vec!["abc0001", "abc0002"]);
    }

    #[test]
    fn test_toggle_drop() {
        let mut state = ReorderTuiState::new(make_entries(3));
        assert!(state.dropped.is_empty());

        // Drop commit at cursor 0
        state.toggle_drop();
        assert!(state.dropped.contains(&0));
        assert!(state.modified);

        // Undrop it
        state.toggle_drop();
        assert!(!state.dropped.contains(&0));
    }

    #[test]
    fn test_cannot_drop_all() {
        let mut state = ReorderTuiState::new(make_entries(2));

        // Drop first
        state.cursor = 0;
        state.toggle_drop();
        assert!(state.dropped.contains(&0));

        // Try to drop second — should be blocked
        state.cursor = 1;
        state.toggle_drop();
        assert!(!state.dropped.contains(&1));
        assert_eq!(state.dropped.len(), 1);
    }

    #[test]
    fn test_get_order_excludes_dropped() {
        let mut state = ReorderTuiState::new(make_entries(3));
        // Drop the middle entry
        state.cursor = 1;
        state.toggle_drop();

        assert_eq!(state.get_order(), vec!["abc0001", "abc0003"]);
    }

    #[test]
    fn test_drop_key_d() {
        let mut state = ReorderTuiState::new(make_entries(3));
        handle_key(&mut state, KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(state.dropped.contains(&0));

        // Press d again to undrop
        handle_key(&mut state, KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(!state.dropped.contains(&0));
    }

    #[test]
    fn test_drop_key_delete() {
        let mut state = ReorderTuiState::new(make_entries(3));
        handle_key(&mut state, KeyCode::Delete, KeyModifiers::NONE);
        assert!(state.dropped.contains(&0));
    }

    #[test]
    fn test_move_preserves_drop_state() {
        let mut state = ReorderTuiState::new(make_entries(3));
        // Drop entry at index 0 (commit 1)
        state.cursor = 0;
        state.toggle_drop();
        assert!(state.dropped.contains(&0));

        // Move it down — drop mark follows the entry to index 1
        state.move_down();
        assert!(!state.dropped.contains(&0));
        assert!(state.dropped.contains(&1));
        // cursor followed the entry to index 1
        assert_eq!(state.cursor, 1);
        // The dropped entry is commit 1, now at index 1
        assert_eq!(state.entries[1].short_sha, "abc0001");
    }
}
