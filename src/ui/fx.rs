//! Visual-effects state machine for the typing screen.
//!
//! This module is intentionally free of any rendering dependency: it derives
//! effect state (afterglows, newline trails, no-miss streak tiers) purely from
//! successive [`TypingSnapshot`]s, so the typing engine stays untouched and
//! effects can never alter core behavior.

use crate::config::FxIntensity;
use crate::domain::typing::TypingSnapshot;

/// Base afterglow lifetime for a freshly typed character.
pub const GLOW_BASE_MS: u64 = 120;
/// Extra afterglow lifetime granted per streak tier.
pub const GLOW_TIER_BONUS_MS: u64 = 40;
/// Lifetime of the lightning-like trail emitted when a line is completed.
pub const TRAIL_MS: u64 = 150;
/// Consecutive accepted keystrokes required for tiers 1..=3.
pub const TIER_THRESHOLDS: [u32; 3] = [30, 100, 250];

/// Fading highlight left behind a typed character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Afterglow {
    pub index: usize,
    pub start_ms: u64,
    pub until_ms: u64,
}

/// Sweeping highlight across a just-completed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineTrail {
    pub line: usize,
    pub start_ms: u64,
    pub until_ms: u64,
}

#[derive(Debug, Clone)]
pub struct FxState {
    prev_cursor: Option<usize>,
    prev_misses: u32,
    streak: u32,
    glows: Vec<Afterglow>,
    trails: Vec<LineTrail>,
    intensity: FxIntensity,
}

impl Default for FxState {
    fn default() -> Self {
        Self {
            prev_cursor: None,
            prev_misses: 0,
            streak: 0,
            glows: Vec::new(),
            trails: Vec::new(),
            intensity: FxIntensity::Normal,
        }
    }
}

impl FxState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_intensity(intensity: FxIntensity) -> Self {
        Self {
            intensity,
            ..Self::default()
        }
    }

    pub fn set_intensity(&mut self, intensity: FxIntensity) {
        self.intensity = intensity;
    }

    /// Current no-miss streak (accepted keystrokes since the last miss).
    pub fn streak(&self) -> u32 {
        self.streak
    }

    /// Streak tier 0..=3.
    pub fn tier(&self) -> u8 {
        TIER_THRESHOLDS
            .iter()
            .filter(|&&t| self.streak >= t)
            .count() as u8
    }

    fn scale_ms(&self, base: u64) -> u64 {
        let scale = self.intensity.lifetime_scale();
        if scale <= 0.0 {
            0
        } else {
            ((base as f32) * scale).round() as u64
        }
    }

    /// Ingest the latest snapshot, deriving keystroke / newline / miss events
    /// from the difference with the previously observed snapshot.
    pub fn observe(&mut self, snap: &TypingSnapshot, now_ms: u64) {
        if self.intensity == FxIntensity::Off {
            self.prev_cursor = Some(snap.cursor);
            self.prev_misses = snap.misses;
            self.glows.clear();
            self.trails.clear();
            return;
        }

        if snap.misses > self.prev_misses {
            self.streak = 0;
        }
        self.prev_misses = snap.misses;

        match self.prev_cursor {
            None => {
                // First observation of a session: auto-indent may already have
                // advanced the cursor; do not emit effects for it.
            }
            Some(prev) if snap.cursor > prev => {
                let glow_ms = self.scale_ms(GLOW_BASE_MS + GLOW_TIER_BONUS_MS * self.tier() as u64);
                let trail_ms = self.scale_ms(TRAIL_MS);
                let chars: Vec<char> = snap.target.chars().collect();
                for idx in prev..snap.cursor {
                    let auto = snap.auto_inserted.binary_search(&idx).is_ok();
                    if !auto {
                        self.streak = self.streak.saturating_add(1);
                        if glow_ms > 0 {
                            self.glows.push(Afterglow {
                                index: idx,
                                start_ms: now_ms,
                                until_ms: now_ms + glow_ms,
                            });
                        }
                    }
                    if chars.get(idx) == Some(&'\n') && trail_ms > 0 {
                        let line = chars[..idx].iter().filter(|&&c| c == '\n').count();
                        self.trails.push(LineTrail {
                            line,
                            start_ms: now_ms,
                            until_ms: now_ms + trail_ms,
                        });
                    }
                }
            }
            Some(prev) if snap.cursor < prev => {
                // Backspace: drop glows for positions no longer typed.
                self.glows.retain(|g| g.index < snap.cursor);
            }
            _ => {}
        }
        self.prev_cursor = Some(snap.cursor);

        self.prune(now_ms);
    }

    /// Drop expired effects.
    pub fn prune(&mut self, now_ms: u64) {
        self.glows.retain(|g| g.until_ms > now_ms);
        self.trails.retain(|t| t.until_ms > now_ms);
    }

    /// Remaining glow intensity (1.0 fresh, 0.0 expired) for a character.
    pub fn glow_intensity(&self, index: usize, now_ms: u64) -> Option<f32> {
        self.glows
            .iter()
            .rev()
            .find(|g| g.index == index && g.until_ms > now_ms)
            .map(|g| remaining(g.start_ms, g.until_ms, now_ms))
    }

    /// Active trail for a display line: `(progress 0..1, intensity 1..0)`.
    /// Progress drives the sweep position, intensity the brightness.
    pub fn trail_at(&self, line: usize, now_ms: u64) -> Option<(f32, f32)> {
        self.trails
            .iter()
            .rev()
            .find(|t| t.line == line && t.until_ms > now_ms)
            .map(|t| {
                let left = remaining(t.start_ms, t.until_ms, now_ms);
                (1.0 - left, left)
            })
    }
}

fn remaining(start_ms: u64, until_ms: u64, now_ms: u64) -> f32 {
    let total = until_ms.saturating_sub(start_ms).max(1) as f32;
    let left = until_ms.saturating_sub(now_ms) as f32;
    (left / total).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::typing::{SessionState, TypingSnapshot};

    fn snap(target: &str, cursor: usize, misses: u32, auto: Vec<usize>) -> TypingSnapshot {
        TypingSnapshot {
            state: SessionState::Active,
            target: target.to_string(),
            cursor,
            expected: target.chars().nth(cursor),
            miss_until_ms: None,
            keystrokes: 0,
            misses,
            elapsed_ms: 0,
            started_at_ms: 0,
            auto_inserted: auto,
        }
    }

    #[test]
    fn first_observation_emits_no_effects() {
        let mut fx = FxState::new();
        fx.observe(&snap("  ab", 2, 0, vec![0, 1]), 0);
        assert_eq!(fx.streak(), 0);
        assert!(fx.glow_intensity(0, 0).is_none());
        assert!(fx.glow_intensity(1, 0).is_none());
    }

    #[test]
    fn advance_registers_glow_and_streak() {
        let mut fx = FxState::new();
        fx.observe(&snap("ab", 0, 0, vec![]), 0);
        fx.observe(&snap("ab", 1, 0, vec![]), 10);
        assert_eq!(fx.streak(), 1);
        let g = fx.glow_intensity(0, 10).expect("glow registered");
        assert!(g > 0.99);
    }

    #[test]
    fn glow_expires_after_lifetime() {
        let mut fx = FxState::new();
        fx.observe(&snap("ab", 0, 0, vec![]), 0);
        fx.observe(&snap("ab", 1, 0, vec![]), 10);
        assert!(fx.glow_intensity(0, 10 + GLOW_BASE_MS - 1).is_some());
        assert!(fx.glow_intensity(0, 10 + GLOW_BASE_MS).is_none());
    }

    #[test]
    fn auto_inserted_chars_do_not_glow_or_count() {
        let mut fx = FxState::new();
        fx.observe(&snap("a\n  b", 0, 0, vec![2, 3]), 0);
        // Typing 'a' + Enter consumes indices 0..4 (auto-indent 2, 3).
        fx.observe(&snap("a\n  b", 4, 0, vec![2, 3]), 10);
        assert_eq!(fx.streak(), 2); // 'a' and '\n' only
        assert!(fx.glow_intensity(2, 10).is_none());
        assert!(fx.glow_intensity(3, 10).is_none());
    }

    #[test]
    fn newline_registers_line_trail() {
        let mut fx = FxState::new();
        fx.observe(&snap("ab\ncd", 2, 0, vec![]), 0);
        fx.observe(&snap("ab\ncd", 3, 0, vec![]), 10);
        let (progress, intensity) = fx.trail_at(0, 10).expect("trail on line 0");
        assert!(progress < 0.01);
        assert!(intensity > 0.99);
        assert!(fx.trail_at(0, 10 + TRAIL_MS).is_none());
    }

    #[test]
    fn miss_resets_streak() {
        let mut fx = FxState::new();
        fx.observe(&snap("abc", 0, 0, vec![]), 0);
        fx.observe(&snap("abc", 2, 0, vec![]), 10);
        assert_eq!(fx.streak(), 2);
        fx.observe(&snap("abc", 2, 1, vec![]), 20);
        assert_eq!(fx.streak(), 0);
    }

    #[test]
    fn tier_thresholds() {
        let mut fx = FxState::new();
        fx.streak = TIER_THRESHOLDS[0] - 1;
        assert_eq!(fx.tier(), 0);
        fx.streak = TIER_THRESHOLDS[0];
        assert_eq!(fx.tier(), 1);
        fx.streak = TIER_THRESHOLDS[1];
        assert_eq!(fx.tier(), 2);
        fx.streak = TIER_THRESHOLDS[2];
        assert_eq!(fx.tier(), 3);
    }

    #[test]
    fn higher_tier_extends_glow() {
        let mut fx = FxState::new();
        fx.streak = TIER_THRESHOLDS[2];
        fx.observe(&snap("ab", 0, 0, vec![]), 0);
        fx.observe(&snap("ab", 1, 0, vec![]), 10);
        let long_life = 10 + GLOW_BASE_MS + GLOW_TIER_BONUS_MS * 3 - 1;
        assert!(fx.glow_intensity(0, long_life).is_some());
    }

    #[test]
    fn backspace_drops_glows_beyond_cursor() {
        let mut fx = FxState::new();
        fx.observe(&snap("abc", 0, 0, vec![]), 0);
        fx.observe(&snap("abc", 2, 0, vec![]), 10);
        assert!(fx.glow_intensity(1, 10).is_some());
        fx.observe(&snap("abc", 1, 0, vec![]), 20);
        assert!(fx.glow_intensity(1, 20).is_none());
        assert!(fx.glow_intensity(0, 20).is_some());
    }
}
