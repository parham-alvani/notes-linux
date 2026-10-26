//! The window's own colors: Daftar's naz palette on dark and Daylight on
//! light, the ones `web/src/reader.css` gives the page, for the parts drawn
//! with GTK (the Today page and the editor's page). Swapped with the scheme.

use gtk::gdk;

/// The page's palette, by name: background, text, muted, and the colors.
const DAYLIGHT: [(&str, &str); 11] = [
    ("background", "#faf9f5"),
    ("text", "#2a2a28"),
    ("muted", "#6a6a62"),
    ("accent", "#a85200"),
    ("red", "#c01030"),
    ("blue", "#0070a0"),
    ("aqua", "#00707a"),
    ("purple", "#8040a0"),
    ("pink", "#b02a70"),
    ("green", "#0a7a48"),
    ("rule", "#d8d5cc"),
];

const NAZ: [(&str, &str); 11] = [
    ("background", "#323232"),
    ("text", "#f5f5f0"),
    ("muted", "#a8a8a0"),
    ("accent", "#ffb040"),
    ("red", "#ff5070"),
    ("blue", "#40d0ff"),
    ("aqua", "#80e5ff"),
    ("purple", "#f0a0f0"),
    ("pink", "#ff69b4"),
    ("green", "#20ffa0"),
    ("rule", "#4a4a4a"),
];

/// The colors a card or a chip may take, as `t-` classes: `accent` alone is
/// libadwaita's own class.
pub const TINTS: [&str; 7] = ["accent", "red", "blue", "aqua", "purple", "pink", "green"];

fn css(palette: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, color) in palette {
        out.push_str(&format!("@define-color daftar_{name} {color};\n"));
    }
    for tint in TINTS {
        out.push_str(&format!(
            ".daftar-card.t-{tint} {{ background-image: linear-gradient(135deg, alpha(@daftar_{tint}, 0.10), alpha(@daftar_{tint}, 0.03)); \
             border: 1px solid alpha(@daftar_{tint}, 0.25); }}\n\
             .daftar-card.t-{tint} .card-title, .t-{tint}.tint {{ color: @daftar_{tint}; }}\n\
             .daftar-card.t-{tint} .card-icon {{ background-image: linear-gradient(to bottom, @daftar_{tint}, alpha(@daftar_{tint}, 0.7)); }}\n\
             .daftar-chip.t-{tint}, .daftar-count.t-{tint}, .daftar-day.t-{tint} {{ color: @daftar_{tint}; background-color: alpha(@daftar_{tint}, 0.14); }}\n\
             .daftar-chip.t-{tint} {{ border: 1px solid alpha(@daftar_{tint}, 0.3); }}\n\
             .t-{tint} checkbutton check {{ border-color: @daftar_{tint}; }}\n\
             .t-{tint} checkbutton check:indeterminate, .t-{tint} checkbutton check:checked {{ background-color: @daftar_{tint}; color: white; }}\n"
        ));
    }
    out.push_str(
        ".daftar-page, .daftar-page > viewport, .daftar-editor { background-color: @daftar_background; }\n\
         .daftar-card { border-radius: 14px; padding: 16px; }\n\
         .daftar-card .card-title { font-weight: bold; }\n\
         .daftar-card .card-icon { border-radius: 7px; color: white; min-width: 26px; min-height: 26px; }\n\
         .daftar-chip { border-radius: 999px; padding: 4px 10px; font-weight: 500; }\n\
         .daftar-count, .daftar-day { border-radius: 999px; padding: 1px 7px; font-weight: bold; font-size: smaller; }\n\
         .daftar-date { font-size: 28pt; font-weight: 800; color: @daftar_accent; }\n\
         .daftar-note { color: @daftar_aqua; }\n\
         .daftar-muted { color: @daftar_muted; }\n\
         .daftar-task { padding: 2px 0; }\n\
         .daftar-task button.flat { padding: 2px 4px; }\n",
    );
    out
}

/// Loads the palette for the current scheme, and again when it changes.
pub fn install() {
    let Some(display) = gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    let manager = adw::StyleManager::default();
    let load = {
        let provider = provider.clone();
        move |dark: bool| provider.load_from_string(&css(if dark { &NAZ } else { &DAYLIGHT }))
    };
    load(manager.is_dark());
    manager.connect_dark_notify(move |manager| load(manager.is_dark()));
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tint_has_a_color_in_both_palettes() {
        for tint in TINTS {
            assert!(NAZ.iter().any(|(name, _)| *name == tint), "{tint}");
            assert!(DAYLIGHT.iter().any(|(name, _)| *name == tint), "{tint}");
        }
        assert!(css(&NAZ).contains("@define-color daftar_accent #ffb040;"));
    }
}
