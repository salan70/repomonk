//! Terminal lifecycle: raw mode + alternate screen with RAII restore.

use std::io::{self, Stdout};

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::Error;

pub type Term = Terminal<CrosstermBackend<Stdout>>;

/// Guards raw mode and alternate screen; restores on drop.
pub struct TerminalGuard {
    terminal: Term,
    restored: bool,
}

impl TerminalGuard {
    pub fn enter() -> crate::Result<Self> {
        enable_raw_mode().map_err(|e| Error::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| Error::Terminal(e.to_string()))?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).map_err(|e| Error::Terminal(e.to_string()))?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }

    pub fn terminal(&mut self) -> &mut Term {
        &mut self.terminal
    }

    pub fn restore(&mut self) -> crate::Result<()> {
        if self.restored {
            return Ok(());
        }
        disable_raw_mode().map_err(|e| Error::Terminal(e.to_string()))?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| Error::Terminal(e.to_string()))?;
        self.terminal
            .show_cursor()
            .map_err(|e| Error::Terminal(e.to_string()))?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
