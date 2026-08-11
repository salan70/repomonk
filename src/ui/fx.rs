//! Visual-effects state machine for the typing screen.
//!
//! This module is intentionally free of any rendering dependency: it derives
//! effect state (afterglows, cursor trails, ripples, and no-miss streak tiers)
//! purely from successive [`TypingSnapshot`]s, so the typing engine stays
//! untouched and effects can never alter core behavior.

use crate::config::{FxIntensity, FxPreset};
use crate::domain::typing::TypingSnapshot;

/// Base afterglow lifetime for a freshly typed character.
pub const GLOW_BASE_MS: u64 = 120;
/// Extra afterglow lifetime granted per streak tier.
pub const GLOW_TIER_BONUS_MS: u64 = 40;
/// Lifetime of the lightning-like trail emitted when a line is completed.
pub const TRAIL_MS: u64 = 150;
/// Consecutive accepted keystrokes required for tiers 1..=3.
pub const TIER_THRESHOLDS: [u32; 3] = [30, 100, 250];
/// Lifetime of a ripple ring emitted by the ripple preset.
pub const RIPPLE_BASE_MS: u64 = 420;
/// Lifetime of a cursor trail in the blaze preset.
pub const BLAZE_TRAIL_BASE_MS: u64 = 300;
/// Lifetime of a cursor trail in the smear preset.
pub const SMEAR_TRAIL_BASE_MS: u64 = 600;
/// Lifetime of the small cursor trail retained by classic and ripple.
pub const SHORT_CURSOR_TRAIL_BASE_MS: u64 = 100;
/// Maximum number of cursor segments retained at once.
pub const CURSOR_SEGMENT_LIMIT: usize = 32;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ripple {
    origin: usize,
    line: usize,
    column: usize,
    start_ms: u64,
    until_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorCell {
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorSegment {
    origin: usize,
    from: CursorCell,
    to: CursorCell,
    start_ms: u64,
    until_ms: u64,
}

#[derive(Debug, Clone)]
pub struct FxState {
    prev_cursor: Option<usize>,
    prev_cell: Option<CursorCell>,
    prev_misses: u32,
    streak: u32,
    glows: Vec<Afterglow>,
    trails: Vec<LineTrail>,
    ripples: Vec<Ripple>,
    cursor_segments: Vec<CursorSegment>,
    intensity: FxIntensity,
    preset: FxPreset,
}

impl Default for FxState {
    fn default() -> Self {
        Self {
            prev_cursor: None,
            prev_cell: None,
            prev_misses: 0,
            streak: 0,
            glows: Vec::new(),
            trails: Vec::new(),
            ripples: Vec::new(),
            cursor_segments: Vec::new(),
            intensity: FxIntensity::Normal,
            preset: FxPreset::Classic,
        }
    }
}

impl FxState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_intensity(intensity: FxIntensity) -> Self {
        Self::with_config(intensity, FxPreset::Classic)
    }

    pub fn with_config(intensity: FxIntensity, preset: FxPreset) -> Self {
        Self {
            intensity,
            preset,
            ..Self::default()
        }
    }

    pub fn set_intensity(&mut self, intensity: FxIntensity) {
        self.intensity = intensity;
        if intensity == FxIntensity::Off {
            self.clear_transient();
        }
    }

    pub fn set_preset(&mut self, preset: FxPreset) {
        if self.preset != preset {
            self.clear_transient();
        }
        self.preset = preset;
    }

    pub fn preset(&self) -> FxPreset {
        self.preset
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
        let chars: Vec<char> = snap.target.chars().collect();
        let current_cell = cursor_cell_at(&chars, snap.cursor);

        if self.intensity == FxIntensity::Off {
            self.prev_cursor = Some(snap.cursor);
            self.prev_cell = Some(current_cell);
            self.prev_misses = snap.misses;
            self.streak = 0;
            self.clear_transient();
            return;
        }

        if snap.misses > self.prev_misses {
            self.streak = 0;
            self.ripples.clear();
            self.cursor_segments.clear();
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
                let ripple_ms = self.scale_ms(RIPPLE_BASE_MS);
                let cursor_trail_ms = self.scale_ms(self.cursor_trail_base_ms());
                let previous_cell = self
                    .prev_cell
                    .unwrap_or_else(|| cursor_cell_at(&chars, prev));
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
                        let from = if idx == prev {
                            previous_cell
                        } else {
                            cursor_cell_at(&chars, idx)
                        };
                        let to = cursor_cell_at(&chars, idx + 1);
                        self.register_cursor_segment(idx, from, to, now_ms, cursor_trail_ms);
                        self.register_ripple(idx, &chars, now_ms, ripple_ms);
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
                self.ripples.retain(|ripple| ripple.origin < snap.cursor);
                self.cursor_segments
                    .retain(|segment| segment.origin < snap.cursor);
            }
            _ => {}
        }
        self.prev_cursor = Some(snap.cursor);
        self.prev_cell = Some(current_cell);

        self.prune(now_ms);
    }

    /// Drop expired effects.
    pub fn prune(&mut self, now_ms: u64) {
        self.glows.retain(|g| g.until_ms > now_ms);
        self.trails.retain(|t| t.until_ms > now_ms);
        self.ripples.retain(|ripple| ripple.until_ms > now_ms);
        self.cursor_segments
            .retain(|segment| segment.until_ms > now_ms);
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

    /// Remaining ring intensity around a cell for the ripple preset.
    pub fn ripple_at(&self, line: usize, column: usize, now_ms: u64) -> Option<f32> {
        if self.preset != FxPreset::Ripple {
            return None;
        }
        self.ripples
            .iter()
            .filter(|ripple| ripple.until_ms > now_ms)
            .filter_map(|ripple| {
                let total = ripple.until_ms.saturating_sub(ripple.start_ms).max(1) as f32;
                let progress =
                    (now_ms.saturating_sub(ripple.start_ms) as f32 / total).clamp(0.0, 1.0);
                let radius = progress * 10.0;
                let line_distance = line.abs_diff(ripple.line) as f32;
                let column_distance = column.abs_diff(ripple.column) as f32;
                let distance =
                    (line_distance * line_distance + column_distance * column_distance).sqrt();
                let ring = (1.0 - (distance - radius).abs() / 2.2).clamp(0.0, 1.0);
                if ring <= 0.0 {
                    return None;
                }
                let fade = (1.0 - progress).max(0.15);
                Some(ring * fade)
            })
            .max_by(f32::total_cmp)
    }

    /// Remaining intensity for a cell crossed by a cursor movement.
    pub fn cursor_trail_at(&self, line: usize, column: usize, now_ms: u64) -> Option<f32> {
        let strength = match self.preset {
            FxPreset::Classic => 0.12,
            FxPreset::Blaze => 1.0,
            FxPreset::Smear => 0.9,
            FxPreset::Ripple => 0.22,
        };
        if strength == 0.0 {
            return None;
        }

        self.cursor_segments
            .iter()
            .filter(|segment| segment.until_ms > now_ms)
            .filter_map(|segment| {
                let path_strength = segment_path_strength(segment, line, column)?;
                Some(
                    path_strength
                        * remaining(segment.start_ms, segment.until_ms, now_ms)
                        * strength,
                )
            })
            .max_by(f32::total_cmp)
    }

    /// Additional bloom intensity for the current cursor cell.
    pub fn cursor_bloom(&self, now_ms: u64) -> f32 {
        let strength = match self.preset {
            FxPreset::Classic => 0.2,
            FxPreset::Blaze => 1.0,
            FxPreset::Smear => 0.65,
            FxPreset::Ripple => 0.35,
        };
        self.cursor_segments
            .iter()
            .rev()
            .find(|segment| segment.until_ms > now_ms)
            .map(|segment| {
                let fade = remaining(segment.start_ms, segment.until_ms, now_ms);
                (fade * strength).clamp(0.0, 1.0)
            })
            .unwrap_or(0.0)
    }

    fn cursor_trail_base_ms(&self) -> u64 {
        match self.preset {
            FxPreset::Classic | FxPreset::Ripple => SHORT_CURSOR_TRAIL_BASE_MS,
            FxPreset::Blaze => BLAZE_TRAIL_BASE_MS,
            FxPreset::Smear => SMEAR_TRAIL_BASE_MS,
        }
    }

    fn register_cursor_segment(
        &mut self,
        index: usize,
        from: CursorCell,
        to: CursorCell,
        now_ms: u64,
        trail_ms: u64,
    ) {
        if trail_ms == 0 {
            return;
        }
        self.cursor_segments.push(CursorSegment {
            origin: index,
            from,
            to,
            start_ms: now_ms,
            until_ms: now_ms + trail_ms,
        });
        if self.cursor_segments.len() > CURSOR_SEGMENT_LIMIT {
            let excess = self.cursor_segments.len() - CURSOR_SEGMENT_LIMIT;
            self.cursor_segments.drain(..excess);
        }
    }

    fn register_ripple(&mut self, index: usize, chars: &[char], now_ms: u64, ripple_ms: u64) {
        if self.preset != FxPreset::Ripple || ripple_ms == 0 {
            return;
        }
        let line = line_index_at(chars, index);
        let column = column_index_at(chars, index);
        self.ripples.push(Ripple {
            origin: index,
            line,
            column,
            start_ms: now_ms,
            until_ms: now_ms + ripple_ms,
        });
    }

    fn clear_transient(&mut self) {
        self.glows.clear();
        self.trails.clear();
        self.ripples.clear();
        self.cursor_segments.clear();
    }
}

fn line_index_at(chars: &[char], index: usize) -> usize {
    chars[..index.min(chars.len())]
        .iter()
        .filter(|&&ch| ch == '\n')
        .count()
}

fn column_index_at(chars: &[char], index: usize) -> usize {
    let index = index.min(chars.len());
    chars[..index]
        .iter()
        .rposition(|&ch| ch == '\n')
        .map_or(index, |newline| index - newline - 1)
}

fn cursor_cell_at(chars: &[char], index: usize) -> CursorCell {
    CursorCell {
        line: line_index_at(chars, index),
        column: column_index_at(chars, index),
    }
}

fn segment_path_strength(segment: &CursorSegment, line: usize, column: usize) -> Option<f32> {
    if segment.from.line == segment.to.line {
        if line != segment.from.line {
            return None;
        }
        let start = segment.from.column.min(segment.to.column);
        let end = segment.from.column.max(segment.to.column);
        if !(start..=end).contains(&column) {
            return None;
        }
        let span = end.saturating_sub(start).max(1) as f32;
        let distance_from_head = segment.to.column.abs_diff(column) as f32;
        return Some((1.0 - distance_from_head / (span + 1.0)).clamp(0.35, 1.0));
    }

    if (line == segment.from.line && column == segment.from.column)
        || (line == segment.to.line && column == segment.to.column)
    {
        Some(0.9)
    } else {
        None
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

    #[test]
    fn ripple_preset_registers_and_expires_rings() {
        let mut fx = FxState::with_config(FxIntensity::Normal, FxPreset::Ripple);
        fx.observe(&snap("abc", 0, 0, vec![]), 0);
        fx.observe(&snap("abc", 1, 0, vec![]), 10);

        assert!(fx.ripple_at(0, 0, 10).is_some());
        assert!(fx.ripple_at(0, 0, 10 + RIPPLE_BASE_MS).is_none());
    }

    #[test]
    fn blaze_preset_registers_cursor_trail_and_bloom() {
        let mut fx = FxState::with_config(FxIntensity::Normal, FxPreset::Blaze);
        fx.observe(&snap("abc", 0, 0, vec![]), 0);
        fx.observe(&snap("abc", 1, 0, vec![]), 10);

        assert!(fx.cursor_trail_at(0, 0, 10).is_some());
        assert!(fx.cursor_trail_at(0, 1, 10).is_some());
        assert!(fx.cursor_bloom(10) > 0.99);
        assert!(fx.cursor_trail_at(0, 0, 10 + BLAZE_TRAIL_BASE_MS).is_none());
    }

    #[test]
    fn smear_preset_keeps_a_long_cursor_trail() {
        let mut fx = FxState::with_config(FxIntensity::Normal, FxPreset::Smear);
        fx.observe(&snap("abc", 0, 0, vec![]), 0);
        fx.observe(&snap("abc", 1, 0, vec![]), 10);

        assert!(fx.cursor_trail_at(0, 0, 10 + BLAZE_TRAIL_BASE_MS).is_some());
        assert!(fx.cursor_trail_at(0, 0, 10 + SMEAR_TRAIL_BASE_MS).is_none());
    }

    #[test]
    fn cursor_trail_clears_on_miss() {
        let mut fx = FxState::with_config(FxIntensity::Normal, FxPreset::Blaze);
        fx.observe(&snap("abc", 0, 0, vec![]), 0);
        fx.observe(&snap("abc", 1, 0, vec![]), 10);
        assert!(fx.cursor_trail_at(0, 0, 10).is_some());

        fx.observe(&snap("abc", 1, 1, vec![]), 20);
        assert!(fx.cursor_trail_at(0, 0, 20).is_none());
        assert_eq!(fx.cursor_bloom(20), 0.0);
    }

    #[test]
    fn off_intensity_clears_preset_effects() {
        let mut fx = FxState::with_config(FxIntensity::Normal, FxPreset::Ripple);
        fx.observe(&snap("abc", 0, 0, vec![]), 0);
        fx.observe(&snap("abc", 1, 0, vec![]), 10);
        fx.set_intensity(FxIntensity::Off);

        assert!(fx.ripple_at(0, 0, 10).is_none());
        assert!(fx.cursor_trail_at(0, 0, 10).is_none());
        assert_eq!(fx.streak(), 1);
    }
}
