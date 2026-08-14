//! Terminal lifecycle: raw mode + alternate screen with RAII restore.

use std::io::{self, Stdout};

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::Error;

pub type Term = Terminal<CrosstermBackend<Stdout>>;

/// Guards raw mode and alternate screen; restores on drop.
pub struct TerminalGuard {
    terminal: Term,
    keyboard_enhancement_enabled: bool,
    restored: bool,
}

impl TerminalGuard {
    pub fn enter() -> crate::Result<Self> {
        enable_raw_mode().map_err(|e| Error::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        let keyboard_enhancement_enabled = matches!(supports_keyboard_enhancement(), Ok(true));
        if keyboard_enhancement_enabled {
            execute!(
                stdout,
                PushKeyboardEnhancementFlags(keyboard_repeat_flags())
            )
            .map_err(|e| Error::Terminal(e.to_string()))?;
        }
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| Error::Terminal(e.to_string()))?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).map_err(|e| Error::Terminal(e.to_string()))?;
        Ok(Self {
            terminal,
            keyboard_enhancement_enabled,
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
        if self.keyboard_enhancement_enabled {
            execute!(self.terminal.backend_mut(), PopKeyboardEnhancementFlags)
                .map_err(|e| Error::Terminal(e.to_string()))?;
        }
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

fn keyboard_repeat_flags() -> KeyboardEnhancementFlags {
    // REPORT_ALL_KEYS_AS_ESCAPE_CODES is required for Repeat events on plain-text
    // keys such as j and k, not only on special keys.
    KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_enhancement_reports_plain_text_key_repeats() {
        let flags = keyboard_repeat_flags();
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES));
    }
}
