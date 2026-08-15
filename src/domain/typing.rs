//! I/O-free typing state machine.

use crate::domain::content::{TypingCheckpoint, TypingMetrics};

/// Normalized input from the terminal layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypingCommand {
    Char(char),
    Enter,
    Backspace,
    Escape,
    /// Advance clock for miss highlight expiry (and elapsed time).
    Tick {
        now_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Active,
    Completed,
    Interrupted,
}

/// Snapshot suitable for UI rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct TypingSnapshot {
    pub state: SessionState,
    /// Full normalized target text.
    pub target: String,
    /// Number of accepted characters (including auto-indent and newlines).
    pub cursor: usize,
    /// Expected next character, if any.
    pub expected: Option<char>,
    /// Until when (ms) the expected char should flash red.
    pub miss_until_ms: Option<u64>,
    pub keystrokes: u32,
    pub misses: u32,
    pub elapsed_ms: u64,
    pub started_at_ms: u64,
    /// Indices that were auto-inserted (indent); excluded from keystroke counts.
    pub auto_inserted: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypingRestoreError {
    CursorOutOfBounds { cursor: usize, target_len: usize },
    CursorInsideAutoIndent { cursor: usize },
}

/// Forced-correction typing engine with auto-indent.
#[derive(Debug, Clone)]
pub struct TypingEngine {
    target: Vec<char>,
    /// Positions that were auto-inserted and must not count as keystrokes.
    auto_inserted: Vec<bool>,
    cursor: usize,
    keystrokes: u32,
    misses: u32,
    state: SessionState,
    miss_until_ms: Option<u64>,
    started_at_ms: u64,
    now_ms: u64,
    allow_backspace: bool,
    auto_indent: bool,
    tab_width: usize,
}

const MISS_FLASH_MS: u64 = 150;

impl TypingEngine {
    /// Create an engine for `normalized` chunk text.
    ///
    /// When `auto_indent` is true, leading indentation on each line is
    /// auto-inserted when the cursor reaches the start of that line.
    pub fn new(
        normalized: &str,
        started_at_ms: u64,
        allow_backspace: bool,
        auto_indent: bool,
        tab_width: usize,
    ) -> Self {
        let target: Vec<char> = normalized.chars().collect();
        let auto_inserted = vec![false; target.len()];
        let mut engine = Self {
            target,
            auto_inserted,
            cursor: 0,
            keystrokes: 0,
            misses: 0,
            state: SessionState::Active,
            miss_until_ms: None,
            started_at_ms,
            now_ms: started_at_ms,
            allow_backspace,
            auto_indent,
            tab_width: tab_width.max(1),
        };
        engine.apply_auto_indent();
        if engine.cursor >= engine.target.len() {
            engine.state = SessionState::Completed;
        }
        engine
    }

    /// Restore a suspended session without counting time spent outside the app.
    pub fn from_checkpoint(
        normalized: &str,
        now_ms: u64,
        checkpoint: &TypingCheckpoint,
        allow_backspace: bool,
        tab_width: usize,
    ) -> Result<Self, TypingRestoreError> {
        let target: Vec<char> = normalized.chars().collect();
        if checkpoint.cursor > target.len() {
            return Err(TypingRestoreError::CursorOutOfBounds {
                cursor: checkpoint.cursor,
                target_len: target.len(),
            });
        }

        let mut engine = Self {
            target,
            auto_inserted: vec![false; normalized.chars().count()],
            cursor: 0,
            keystrokes: checkpoint.keystrokes,
            misses: checkpoint.misses,
            state: SessionState::Active,
            miss_until_ms: None,
            started_at_ms: now_ms.saturating_sub(checkpoint.elapsed_ms),
            now_ms,
            allow_backspace,
            auto_indent: checkpoint.auto_indent,
            tab_width: tab_width.max(1),
        };

        while engine.cursor < checkpoint.cursor {
            let before = engine.cursor;
            engine.apply_auto_indent();
            if engine.cursor > checkpoint.cursor {
                return Err(TypingRestoreError::CursorInsideAutoIndent {
                    cursor: checkpoint.cursor,
                });
            }
            if engine.cursor == checkpoint.cursor {
                break;
            }
            engine.cursor += 1;
            if engine.cursor == before {
                return Err(TypingRestoreError::CursorOutOfBounds {
                    cursor: checkpoint.cursor,
                    target_len: engine.target.len(),
                });
            }
        }
        engine.apply_auto_indent();
        if engine.cursor > checkpoint.cursor {
            return Err(TypingRestoreError::CursorInsideAutoIndent {
                cursor: checkpoint.cursor,
            });
        }
        if engine.cursor >= engine.target.len() {
            engine.state = SessionState::Completed;
        }
        Ok(engine)
    }

    /// Rebase the clock after a pause so the paused interval is not counted.
    ///
    /// Elapsed time is derived as `now_ms - started_at_ms`, so resuming means
    /// moving the start forward by however long the pause lasted. This is the
    /// same adjustment `from_checkpoint` makes for time spent outside Typing.
    pub fn resume_at(&mut self, now_ms: u64) {
        let elapsed = self.now_ms.saturating_sub(self.started_at_ms);
        self.started_at_ms = now_ms.saturating_sub(elapsed);
        self.now_ms = now_ms;
    }

    pub fn snapshot(&self) -> TypingSnapshot {
        let expected = self.target.get(self.cursor).copied();
        let auto_idxs: Vec<usize> = self
            .auto_inserted
            .iter()
            .enumerate()
            .filter_map(|(i, flag)| flag.then_some(i))
            .collect();
        TypingSnapshot {
            state: self.state,
            target: self.target.iter().collect(),
            cursor: self.cursor,
            expected,
            miss_until_ms: self.miss_until_ms,
            keystrokes: self.keystrokes,
            misses: self.misses,
            elapsed_ms: self.now_ms.saturating_sub(self.started_at_ms),
            started_at_ms: self.started_at_ms,
            auto_inserted: auto_idxs,
        }
    }

    pub fn metrics(&self) -> TypingMetrics {
        TypingMetrics::from_counts(
            self.keystrokes,
            self.misses,
            self.now_ms.saturating_sub(self.started_at_ms),
        )
    }

    pub fn apply(&mut self, cmd: TypingCommand) {
        if self.state != SessionState::Active {
            return;
        }
        match cmd {
            TypingCommand::Tick { now_ms } => {
                self.now_ms = now_ms;
                if let Some(until) = self.miss_until_ms {
                    if now_ms >= until {
                        self.miss_until_ms = None;
                    }
                }
            }
            TypingCommand::Escape => {
                self.state = SessionState::Interrupted;
            }
            TypingCommand::Backspace => {
                if !self.allow_backspace {
                    return;
                }
                self.handle_backspace();
            }
            TypingCommand::Enter => self.handle_char('\n'),
            TypingCommand::Char(c) => {
                if c == '\n' {
                    self.handle_char('\n');
                } else if c == '\t' {
                    // Tabs are normalized away in content; treat as spaces of tab_width
                    // only if target expects spaces — otherwise miss on first space mismatch.
                    // For simplicity, expand tab into tab_width space attempts.
                    for _ in 0..self.tab_width {
                        if self.state != SessionState::Active {
                            break;
                        }
                        self.handle_char(' ');
                    }
                } else {
                    self.handle_char(c);
                }
            }
        }
    }

    fn handle_char(&mut self, input: char) {
        let Some(&expected) = self.target.get(self.cursor) else {
            return;
        };
        if input == expected {
            self.cursor += 1;
            if !self
                .auto_inserted
                .get(self.cursor - 1)
                .copied()
                .unwrap_or(false)
            {
                self.keystrokes += 1;
            }
            self.miss_until_ms = None;
            self.apply_auto_indent();
            if self.cursor >= self.target.len() {
                self.state = SessionState::Completed;
            }
        } else {
            self.misses += 1;
            self.miss_until_ms = Some(self.now_ms.saturating_add(MISS_FLASH_MS));
        }
    }

    fn handle_backspace(&mut self) {
        // Move back over typed (non-auto) characters only; never erase auto-indent.
        if self.cursor == 0 {
            return;
        }
        let prev = self.cursor - 1;
        if self.auto_inserted.get(prev).copied().unwrap_or(false) {
            return;
        }
        self.cursor = prev;
    }

    /// Auto-insert leading spaces at the beginning of the current line.
    fn apply_auto_indent(&mut self) {
        if !self.auto_indent {
            return;
        }
        if self.cursor >= self.target.len() {
            return;
        }
        let at_line_start =
            self.cursor == 0 || self.target.get(self.cursor - 1).copied() == Some('\n');
        if !at_line_start {
            return;
        }
        while let Some(&ch) = self.target.get(self.cursor) {
            if ch == ' ' {
                if let Some(flag) = self.auto_inserted.get_mut(self.cursor) {
                    *flag = true;
                }
                self.cursor += 1;
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(text: &str) -> TypingEngine {
        TypingEngine::new(text, 0, true, true, 4)
    }

    #[test]
    fn accepts_matching_chars() {
        let mut e = engine("ab");
        e.apply(TypingCommand::Char('a'));
        e.apply(TypingCommand::Char('b'));
        assert_eq!(e.snapshot().state, SessionState::Completed);
        assert_eq!(e.snapshot().keystrokes, 2);
    }

    #[test]
    fn rejects_mismatch_without_advancing() {
        let mut e = engine("ab");
        e.apply(TypingCommand::Char('x'));
        let s = e.snapshot();
        assert_eq!(s.cursor, 0);
        assert_eq!(s.misses, 1);
        assert_eq!(s.miss_until_ms, Some(150));
    }

    #[test]
    fn enter_matches_newline() {
        let mut e = engine("a\nb");
        e.apply(TypingCommand::Char('a'));
        e.apply(TypingCommand::Enter);
        e.apply(TypingCommand::Char('b'));
        assert_eq!(e.snapshot().state, SessionState::Completed);
    }

    #[test]
    fn backspace_moves_back_but_keeps_misses() {
        let mut e = engine("ab");
        e.apply(TypingCommand::Char('a'));
        e.apply(TypingCommand::Char('x'));
        e.apply(TypingCommand::Backspace);
        let s = e.snapshot();
        assert_eq!(s.cursor, 0);
        assert_eq!(s.misses, 1);
    }

    #[test]
    fn escape_interrupts() {
        let mut e = engine("ab");
        e.apply(TypingCommand::Escape);
        assert_eq!(e.snapshot().state, SessionState::Interrupted);
    }

    #[test]
    fn auto_indent_skips_leading_spaces() {
        let mut e = engine("  ab");
        let s = e.snapshot();
        assert_eq!(s.cursor, 2);
        assert_eq!(s.expected, Some('a'));
        e.apply(TypingCommand::Char('a'));
        e.apply(TypingCommand::Char('b'));
        assert_eq!(e.snapshot().keystrokes, 2);
        assert_eq!(e.snapshot().state, SessionState::Completed);
    }

    #[test]
    fn auto_indent_after_newline() {
        let mut e = engine("a\n  b");
        e.apply(TypingCommand::Char('a'));
        e.apply(TypingCommand::Enter);
        assert_eq!(e.snapshot().cursor, 4); // "a\n  " consumed
        assert_eq!(e.snapshot().expected, Some('b'));
    }

    #[test]
    fn backspace_cannot_erase_auto_indent() {
        let mut e = engine("  ab");
        assert_eq!(e.snapshot().cursor, 2);
        e.apply(TypingCommand::Backspace);
        assert_eq!(e.snapshot().cursor, 2);
        e.apply(TypingCommand::Char('a'));
        assert_eq!(e.snapshot().cursor, 3);
        e.apply(TypingCommand::Backspace);
        assert_eq!(e.snapshot().cursor, 2);
    }

    #[test]
    fn miss_flash_clears_on_tick() {
        let mut e = engine("a");
        e.apply(TypingCommand::Char('x'));
        e.apply(TypingCommand::Tick { now_ms: 150 });
        assert_eq!(e.snapshot().miss_until_ms, None);
    }

    #[test]
    fn resume_at_does_not_charge_the_paused_interval() {
        let mut e = engine("ab");
        e.apply(TypingCommand::Tick { now_ms: 1_000 });
        assert_eq!(e.snapshot().elapsed_ms, 1_000);
        // Paused for ten seconds: no ticks arrive, so the clock is frozen, and
        // resuming moves the start forward instead of billing the pause.
        e.resume_at(11_000);
        assert_eq!(e.snapshot().elapsed_ms, 1_000);
        e.apply(TypingCommand::Tick { now_ms: 11_500 });
        assert_eq!(e.snapshot().elapsed_ms, 1_500);
    }

    #[test]
    fn metrics_exclude_idle_not_applied_idle_is_included() {
        let mut e = engine("a");
        e.apply(TypingCommand::Tick { now_ms: 60_000 });
        e.apply(TypingCommand::Char('a'));
        let m = e.metrics();
        assert_eq!(m.keystrokes, 1);
        assert!((m.kpm - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn auto_indent_can_be_disabled() {
        let e = TypingEngine::new("  ab", 0, true, false, 4);
        assert_eq!(e.snapshot().cursor, 0);
        assert_eq!(e.snapshot().expected, Some(' '));
    }

    #[test]
    fn checkpoint_restores_cursor_metrics_and_auto_indent() {
        let mut e = TypingEngine::new("a\n  bc", 0, true, true, 4);
        e.apply(TypingCommand::Char('a'));
        e.apply(TypingCommand::Enter);
        e.apply(TypingCommand::Char('b'));
        e.apply(TypingCommand::Tick { now_ms: 1_000 });
        let snapshot = e.snapshot();
        let checkpoint = TypingCheckpoint {
            chunk_id: 7,
            cursor: snapshot.cursor,
            keystrokes: snapshot.keystrokes,
            misses: snapshot.misses,
            elapsed_ms: snapshot.elapsed_ms,
            started_at: "t0".into(),
            auto_indent: true,
        };

        let restored = TypingEngine::from_checkpoint("a\n  bc", 5_000, &checkpoint, true, 4)
            .expect("valid checkpoint");
        let restored_snapshot = restored.snapshot();
        assert_eq!(restored_snapshot.cursor, snapshot.cursor);
        assert_eq!(restored_snapshot.expected, snapshot.expected);
        assert_eq!(restored_snapshot.keystrokes, snapshot.keystrokes);
        assert_eq!(restored_snapshot.misses, snapshot.misses);
        assert_eq!(restored_snapshot.elapsed_ms, snapshot.elapsed_ms);
        assert!(restored_snapshot.auto_inserted.contains(&2));
        assert!(restored_snapshot.auto_inserted.contains(&3));
    }

    #[test]
    fn checkpoint_rejects_cursor_inside_auto_indent() {
        let checkpoint = TypingCheckpoint {
            chunk_id: 1,
            cursor: 1,
            keystrokes: 0,
            misses: 0,
            elapsed_ms: 0,
            started_at: "t0".into(),
            auto_indent: true,
        };
        assert!(matches!(
            TypingEngine::from_checkpoint("  x", 0, &checkpoint, true, 4),
            Err(TypingRestoreError::CursorInsideAutoIndent { .. })
        ));
    }
}
