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
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| Error::Terminal(e.to_string()))?;
        // The keyboard enhancement stack is per-screen, so push only after entering
        // the alternate screen. Pushing on the main screen would leave the flags
        // active there after exit, and the shell would receive CSI u escape codes.
        if keyboard_enhancement_enabled {
            execute!(
                stdout,
                PushKeyboardEnhancementFlags(keyboard_repeat_flags())
            )
            .map_err(|e| Error::Terminal(e.to_string()))?;
        }
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
        // Mark restored up front: every step below is best-effort, and a partial
        // failure must not leave the terminal in raw mode on a retry.
        self.restored = true;
        let mut first_err = None;
        let mut record = |res: io::Result<()>| {
            if let Err(e) = res {
                first_err.get_or_insert(Error::Terminal(e.to_string()));
            }
        };
        // Pop while still on the alternate screen, mirroring the push in `enter`.
        if self.keyboard_enhancement_enabled {
            record(execute!(
                self.terminal.backend_mut(),
                PopKeyboardEnhancementFlags
            ));
        }
        record(execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        ));
        record(self.terminal.show_cursor());
        record(disable_raw_mode());
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

fn keyboard_repeat_flags() -> KeyboardEnhancementFlags {
    // REPORT_ALL_KEYS_AS_ESCAPE_CODES is required for Repeat events on plain-text
    // keys such as j and k, not only on special keys.
    //
    // REPORT_ALTERNATE_KEYS is mandatory alongside it: escape-code reporting sends
    // the unshifted key code, so without the alternate (shifted) key code Shift+a
    // arrives as Char('a') + SHIFT and Shift+9 as Char('9') + SHIFT. Typing would
    // then reject every uppercase letter and shifted symbol.
    KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
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

    #[test]
    fn keyboard_enhancement_reports_shifted_characters() {
        // Without alternate keys, Shift+a would arrive as Char('a') + SHIFT.
        assert!(keyboard_repeat_flags().contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS));
    }
}
