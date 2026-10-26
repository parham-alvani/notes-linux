//! Daftar's colors in a terminal: naz on dark and Daylight on light, the
//! variables of `web/src/reader.css`. The window follows `appearance.theme`;
//! "system" is dark, since a terminal does not say which it is.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background: Color,
    pub panel: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub rule: Color,
    pub accent: Color,
    pub strong: Color,
    pub em: Color,
    pub code: Color,
    pub code_background: Color,
    pub link: Color,
    pub done: Color,
    pub highlight_bg: Color,
    pub highlight_fg: Color,
    pub headings: [Color; 6],
    pub red: Color,
    pub orange: Color,
    pub yellow: Color,
    pub vivid_yellow: Color,
    pub green: Color,
    pub chartreuse: Color,
    pub lime: Color,
    pub blue: Color,
    pub aqua: Color,
    pub purple: Color,
    pub pink: Color,
    pub warm_grey: Color,
}

const fn hex(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Theme {
    /// naz, the dark palette.
    pub const NAZ: Theme = Theme {
        background: hex(0x323232),
        panel: hex(0x3a3a35),
        text: hex(0xf5f5f0),
        muted: hex(0xa8a8a0),
        faint: hex(0x707080),
        rule: hex(0x4a4a4a),
        accent: hex(0xffb040),
        strong: hex(0xffb070),
        em: hex(0xb8ffd0),
        code: hex(0xffd850),
        code_background: hex(0x282826),
        link: hex(0x80e5ff),
        done: hex(0x707080),
        highlight_bg: hex(0xf0e890),
        highlight_fg: hex(0x323232),
        headings: [hex(0xffe0a0), hex(0xffff88), hex(0x60d8ff), hex(0xe0ffd8), hex(0xff69b4), hex(0xf0a0f0)],
        red: hex(0xff5070),
        orange: hex(0xffb040),
        yellow: hex(0xf0e890),
        vivid_yellow: hex(0xffe040),
        green: hex(0x20ffa0),
        chartreuse: hex(0xa0ff40),
        lime: hex(0xb8ffd0),
        blue: hex(0x40d0ff),
        aqua: hex(0x80e5ff),
        purple: hex(0xf0a0f0),
        pink: hex(0xff69b4),
        warm_grey: hex(0xb09070),
    };

    /// Daylight, the light palette.
    pub const DAYLIGHT: Theme = Theme {
        background: hex(0xfaf9f5),
        panel: hex(0xefede6),
        text: hex(0x2a2a28),
        muted: hex(0x6a6a62),
        faint: hex(0x6a6a62),
        rule: hex(0xd8d5cc),
        accent: hex(0xa85200),
        strong: hex(0xa85200),
        em: hex(0x0a7a48),
        code: hex(0x8a6a00),
        code_background: hex(0xefede6),
        link: hex(0x00707a),
        done: hex(0x6a6a62),
        highlight_bg: hex(0x8a6a00),
        highlight_fg: hex(0xfaf9f5),
        headings: [hex(0xa85200), hex(0x8a6a00), hex(0x0070a0), hex(0x0a7a48), hex(0xb02a70), hex(0x8040a0)],
        red: hex(0xc01030),
        orange: hex(0xa85200),
        yellow: hex(0x8a6a00),
        vivid_yellow: hex(0x8a6a00),
        green: hex(0x0a7a48),
        chartreuse: hex(0x5a7a00),
        lime: hex(0x0a7a48),
        blue: hex(0x0070a0),
        aqua: hex(0x00707a),
        purple: hex(0x8040a0),
        pink: hex(0xb02a70),
        warm_grey: hex(0x7a5a3a),
    };

    /// The palette `appearance.theme` names: `light`, else naz.
    pub fn named(name: Option<&str>) -> Theme {
        match name {
            Some("light") => Theme::DAYLIGHT,
            _ => Theme::NAZ,
        }
    }

    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.background)
    }

    pub fn muted(&self) -> Style {
        Style::new().fg(self.muted)
    }

    pub fn faint(&self) -> Style {
        Style::new().fg(self.faint)
    }

    pub fn heading(&self, level: usize) -> Style {
        Style::new().fg(self.headings[level.clamp(1, 6) - 1]).add_modifier(Modifier::BOLD)
    }

    /// A callout's color, by its type, as the reader colors it.
    pub fn callout(&self, kind: &str) -> Color {
        match kind.to_lowercase().as_str() {
            "warning" | "caution" | "attention" => self.orange,
            "danger" | "error" | "bug" | "failure" | "fail" | "missing" => self.red,
            "success" | "check" | "done" => self.green,
            "tip" | "hint" | "important" => self.chartreuse,
            "todo" => self.vivid_yellow,
            "question" | "help" | "faq" => self.yellow,
            "abstract" | "summary" | "tldr" => self.lime,
            "example" => self.purple,
            "quote" | "cite" => self.warm_grey,
            "figure" => self.pink,
            "read" => self.aqua,
            _ => self.blue,
        }
    }

    /// A callout's mark, by its type.
    pub fn callout_icon(kind: &str) -> &'static str {
        match kind.to_lowercase().as_str() {
            "warning" | "caution" | "attention" => "⚠",
            "danger" | "error" | "bug" | "failure" | "fail" | "missing" => "✗",
            "success" | "check" | "done" => "✓",
            "tip" | "hint" | "important" => "✦",
            "todo" => "☐",
            "question" | "help" | "faq" => "?",
            "example" => "≡",
            "quote" | "cite" => "❝",
            _ => "ℹ",
        }
    }
}
