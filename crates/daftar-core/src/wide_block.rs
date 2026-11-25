//! A table, a code block or a picture wider than the prose may grow past
//! the measure into the room beside it (`docs/specs/wide-blocks.md`). The
//! port of `WideBlock.swift`.

/// Where a wide block stands: its width and its start, from the pane's
/// left edge, in the reader's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WideBlock {
    pub width: f64,
    pub offset: f64,
}

impl WideBlock {
    /// The place of a block [natural] px wide, with the measure [measure]
    /// px, in a pane [pane] px wide that keeps [side] px at each side, and
    /// blocks allowed [limit] px (`appearance.block_width` em at the text
    /// size; 0 is off). `None` when the block keeps the measure, as before.
    pub fn place(natural: f64, measure: f64, pane: f64, side: f64, limit: f64) -> Option<WideBlock> {
        if limit <= 0.0 || measure <= 0.0 || natural <= measure {
            return None;
        }
        let most = limit.min(pane - 2.0 * side);
        if most <= measure {
            return None;
        }
        let width = natural.min(most);
        Some(WideBlock { width, offset: (pane - width) / 2.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Size 16, `text_width = 35`, 40 px at each side, `block_width = 60`.
    fn place(natural: f64, pane: f64) -> Option<WideBlock> {
        WideBlock::place(natural, 560.0, pane, 40.0, 960.0)
    }

    #[test]
    fn a_block_that_fits_keeps_the_measure() {
        assert_eq!(place(500.0, 1400.0), None);
        assert_eq!(place(560.0, 1400.0), None);
    }

    #[test]
    fn a_wide_block_grows_to_its_own_width() {
        assert_eq!(place(800.0, 1400.0), Some(WideBlock { width: 800.0, offset: 300.0 }));
    }

    #[test]
    fn the_limit_is_the_setting_or_the_pane() {
        assert_eq!(place(1200.0, 1400.0), Some(WideBlock { width: 960.0, offset: 220.0 }));
        assert_eq!(place(1200.0, 900.0), Some(WideBlock { width: 820.0, offset: 40.0 }));
    }

    #[test]
    fn a_narrow_pane_keeps_the_measure() {
        assert_eq!(place(800.0, 600.0), None);
    }

    #[test]
    fn block_width_zero_is_off() {
        assert_eq!(WideBlock::place(800.0, 560.0, 1400.0, 40.0, 0.0), None);
        // `text_width = 0`: no measure to grow past.
        assert_eq!(WideBlock::place(800.0, 0.0, 1400.0, 40.0, 960.0), None);
    }
}
