//! The window's own colors: Daftar's naz palette on dark and Daylight on
//! light, the ones `web/src/reader.css` gives the page, for the parts drawn
//! with GTK (the Today page, the board and the editor's page). Swapped with the scheme.

use gtk::gdk;

/// The page's palette, by name: background, text, muted, and the colors;
/// `on_accent` is the text on the accent.
const DAYLIGHT: [(&str, &str); 14] = [
    ("panel", "#efede6"),
    ("yellow", "#8a6a00"),
    ("background", "#faf9f5"),
    ("text", "#2a2a28"),
    ("muted", "#6a6a62"),
    ("accent", "#a85200"),
    ("on_accent", "#ffffff"),
    ("red", "#c01030"),
    ("blue", "#0070a0"),
    ("aqua", "#00707a"),
    ("purple", "#8040a0"),
    ("pink", "#b02a70"),
    ("green", "#0a7a48"),
    ("rule", "#d8d5cc"),
];

const NAZ: [(&str, &str); 14] = [
    ("panel", "#3a3a35"),
    ("yellow", "#ffd850"),
    ("background", "#323232"),
    ("text", "#f5f5f0"),
    ("muted", "#a8a8a0"),
    ("accent", "#ffb040"),
    ("on_accent", "#323232"),
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
pub const TINTS: [&str; 9] = ["accent", "red", "blue", "aqua", "purple", "pink", "green", "yellow", "muted"];

fn css(palette: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, color) in palette {
        out.push_str(&format!("@define-color daftar_{name} {color};\n"));
    }
    out.push_str(&adwaita(palette));
    for tint in TINTS {
        out.push_str(&format!(
            ".daftar-card.t-{tint} {{ background-image: linear-gradient(135deg, alpha(@daftar_{tint}, 0.10), alpha(@daftar_{tint}, 0.03)); \
             border: 1px solid alpha(@daftar_{tint}, 0.25); }}\n\
             .daftar-card.t-{tint} .card-title, .t-{tint}.tint {{ color: @daftar_{tint}; }}\n\
             .daftar-card.t-{tint} .card-icon {{ background-image: linear-gradient(to bottom, @daftar_{tint}, alpha(@daftar_{tint}, 0.7)); }}\n\
             .daftar-chip.t-{tint}, .daftar-count.t-{tint}, .daftar-day.t-{tint} {{ color: @daftar_{tint}; background-color: alpha(@daftar_{tint}, 0.14); }}\n\
             .daftar-chip.t-{tint} {{ border: 1px solid alpha(@daftar_{tint}, 0.3); }}\n\
             .t-{tint} checkbutton check {{ border-color: @daftar_{tint}; }}\n\
             .t-{tint} checkbutton check:indeterminate, .t-{tint} checkbutton check:checked {{ background-color: @daftar_{tint}; color: white; }}\n\
             .daftar-meeting.t-{tint} .meeting-bar {{ background-color: @daftar_{tint}; }}\n\
             .daftar-meeting.t-{tint} .tint {{ color: @daftar_{tint}; }}\n\
             .daftar-column.t-{tint} {{ background-color: alpha(@daftar_{tint}, 0.05); }}\n\
             .daftar-column.t-{tint}.drop-target {{ background-color: alpha(@daftar_{tint}, 0.16); border-color: alpha(@daftar_{tint}, 0.8); }}\n\
             .daftar-column.t-{tint} .column-dot {{ background-color: @daftar_{tint}; }}\n\
             .daftar-column.t-{tint} .column-head {{ background-color: alpha(@daftar_{tint}, 0.14); }}\n\
             .t-{tint} .board-card {{ border-left: 3px solid @daftar_{tint}; }}\n\
             .daftar-pill.t-{tint}:checked {{ background-color: @daftar_{tint}; color: @daftar_background; font-weight: bold; }}\n"
        ));
    }
    out.push_str(
        "vte-terminal { padding: 6px 12px; }\n\
         .daftar-split > stack { border-top: 2px solid transparent; }\n\
         .daftar-split > stack.daftar-pane-focused { border-top-color: @daftar_accent; }\n\
         .daftar-page, .daftar-page > viewport, .daftar-editor { background-color: @daftar_background; }\n\
         .daftar-card { border-radius: 14px; padding: 16px; }\n\
         .daftar-card .card-title { font-weight: bold; }\n\
         .daftar-card .card-icon { border-radius: 7px; color: white; min-width: 26px; min-height: 26px; }\n\
         .daftar-chip { border-radius: 999px; padding: 4px 10px; font-weight: 500; }\n\
         .daftar-count, .daftar-day { border-radius: 999px; padding: 1px 7px; font-weight: bold; font-size: smaller; }\n\
         .daftar-date { font-size: 28pt; font-weight: 800; color: @daftar_accent; }\n\
         .daftar-note { color: @daftar_aqua; }\n\
         .daftar-muted { color: @daftar_muted; }\n\
         .daftar-picker { padding: 6px; }\n\
         .daftar-picker-day { min-width: 34px; padding: 2px 0; border-radius: 8px; }\n\
         .daftar-picker-day.weekend { color: @daftar_muted; }\n\
         .daftar-picker-day.holiday { color: @daftar_red; }\n\
         .daftar-picker-day.today { box-shadow: inset 0 0 0 1px @daftar_accent; }\n\
         .daftar-picker-day.selected { background-color: @daftar_accent; color: @daftar_on_accent; }\n\
         .daftar-picker-other { font-size: 7pt; opacity: 0.6; }\n\
         .daftar-picker-dot { font-size: 7pt; color: @daftar_aqua; margin-top: -4px; }\n\
         .daftar-picker-day.holiday .daftar-picker-dot { color: @daftar_red; }\n\
         .daftar-picker-day.selected .daftar-picker-dot { color: @daftar_on_accent; }\n\
         .daftar-task { padding: 2px 0; }\n\
         .daftar-task button.flat { padding: 2px 4px; }\n\
         .daftar-meeting { padding: 3px 0; }\n\
         .daftar-meeting .meeting-bar { border-radius: 2px; min-height: 18px; }\n\
         .daftar-meeting.is-past { opacity: 0.6; }\n\
         .daftar-meeting .meeting-warning { color: @daftar_red; }\n\
         .daftar-badge { border-radius: 999px; padding: 0 6px; font-weight: bold; font-size: smaller; color: white; background-color: @daftar_accent; }\n\
         .daftar-join { background-color: @daftar_blue; color: @daftar_background; font-weight: bold; padding: 2px 12px; min-height: 0; }\n\
         .daftar-join-now { background-color: @daftar_accent; color: @daftar_background; font-weight: bold; padding: 2px 12px; min-height: 0; }\n\
         .daftar-prep { background-color: @daftar_purple; color: @daftar_background; font-weight: bold; padding: 2px 12px; min-height: 0; }\n\
         .daftar-column { border-radius: 16px; padding: 8px; border: 2px solid transparent; }\n\
         .daftar-column .column-head { border-radius: 12px; padding: 8px 12px; }\n\
         .daftar-column .column-dot { border-radius: 999px; min-width: 9px; min-height: 9px; }\n\
         image.folder-logo { border-radius: 3px; }\n\
         .daftar-outline label.outline-h1 { color: @daftar_accent; font-weight: 600; }\n\
         .daftar-outline label.outline-h2 { color: @daftar_yellow; font-weight: 600; }\n\
         .daftar-outline label.outline-h3 { color: @daftar_blue; }\n\
         .daftar-outline label.outline-h4 { color: @daftar_green; }\n\
         .daftar-outline label.outline-h5 { color: @daftar_pink; }\n\
         .daftar-outline label.outline-h6 { color: @daftar_purple; }\n\
         .daftar-tags link { color: @daftar_pink; }\n\
         .daftar-vim-cursor { background-color: alpha(@daftar_accent, 0.45); border-radius: 2px; }\n\
         .daftar-vim-cursor.bar { background-color: @daftar_accent; border-radius: 1px; }\n\
         .daftar-vim-message label { padding: 6px 10px; }\n\
         .board-card { background-color: @daftar_panel; border-radius: 11px; padding: 10px; border: 1px solid alpha(@daftar_text, 0.06); }\n\
         .board-card.is-closed .board-text { color: @daftar_muted; text-decoration: line-through; }\n\
         .board-card:focus-visible { box-shadow: inset 0 0 0 2px @daftar_accent; background-color: alpha(@daftar_accent, 0.08); }\n\
         .daftar-board-head { padding: 10px 16px; }\n\
         .daftar-board-icon { border-radius: 8px; color: white; min-width: 30px; min-height: 30px; background-image: linear-gradient(135deg, @daftar_purple, @daftar_accent); }\n\
         .daftar-pill { border-radius: 999px; padding: 4px 9px; }\n\
         .daftar-home { background-color: @daftar_background; }\n\
         .daftar-home .home-scrim { background-image: linear-gradient(to bottom, alpha(@daftar_background, 0.15), alpha(@daftar_background, 0.3), alpha(@daftar_background, 0.6)); }\n\
         .daftar-home .home-clock { text-shadow: 0 0 10px alpha(@daftar_background, 0.7); color: @daftar_text; }\n\
         .daftar-home .home-time { font-size: 66pt; font-weight: 300; font-feature-settings: \"tnum\"; }\n\
         .daftar-home.compact .home-time { font-size: 36pt; }\n\
         .daftar-home .home-date { font-size: 15pt; color: @daftar_muted; }\n\
         .daftar-home .home-greeting { font-size: 21pt; font-weight: 500; }\n\
         .daftar-home .home-glass { background-color: alpha(@daftar_panel, 0.62); border: 1px solid alpha(@daftar_text, 0.10); box-shadow: 0 4px 18px alpha(black, 0.12); }\n\
         .daftar-home .home-search { border-radius: 999px; padding: 6px 16px; font-size: 13pt; }\n\
         .daftar-home .home-search entry, .daftar-home .home-search text { background: none; border: none; box-shadow: none; outline: none; }\n\
         .daftar-home .home-card { border-radius: 18px; padding: 10px; }\n\
         .daftar-home .home-heading { font-weight: 600; color: @daftar_muted; margin: 0 8px 4px 8px; }\n\
         .daftar-home .home-row { border-radius: 8px; padding: 5px 8px; min-height: 0; }\n\
         .daftar-home .home-row:hover { background-color: alpha(@daftar_text, 0.08); }\n\
         .daftar-home .home-folder, .daftar-home .home-keys { color: alpha(@daftar_muted, 0.8); }\n\
         .daftar-home .home-quote { font-size: 14pt; font-style: italic; }\n",
    );
    // Sync Conflicts, History and Quick Capture.
    out.push_str(
        ".daftar-sheet-icon { border-radius: 11px; color: white; min-width: 42px; min-height: 42px; }\n\
         .daftar-sheet-icon.conflicts { background-image: linear-gradient(135deg, @daftar_accent, @daftar_red); }\n\
         .daftar-sheet-icon.history { background-image: linear-gradient(135deg, @daftar_aqua, @daftar_blue); }\n\
         .daftar-hunk { border-radius: 10px; padding: 12px; background-color: alpha(@daftar_text, 0.05); }\n\
         .daftar-side { border-radius: 8px; padding: 8px; border: 1px solid transparent; }\n\
         .daftar-side.t-blue { background-color: alpha(@daftar_blue, 0.06); }\n\
         .daftar-side.t-blue.kept { background-color: alpha(@daftar_blue, 0.16); border-color: alpha(@daftar_blue, 0.6); }\n\
         .daftar-side.t-accent { background-color: alpha(@daftar_accent, 0.06); }\n\
         .daftar-side.t-accent.kept { background-color: alpha(@daftar_accent, 0.16); border-color: alpha(@daftar_accent, 0.6); }\n\
         .daftar-side-title.t-blue { color: @daftar_blue; font-weight: bold; }\n\
         .daftar-side-title.t-accent { color: @daftar_accent; font-weight: bold; }\n\
         .daftar-diff { border-radius: 10px; background-color: alpha(@daftar_text, 0.05); padding: 6px 0; }\n\
         .daftar-diff .diff-line { padding: 0 8px; }\n\
         .daftar-diff .diff-added { background-color: alpha(@daftar_green, 0.12); color: @daftar_green; }\n\
         .daftar-diff .diff-removed { background-color: alpha(@daftar_red, 0.12); color: @daftar_red; }\n\
         .daftar-diff .diff-number { color: alpha(@daftar_muted, 0.7); }\n\
         .daftar-added-count { color: @daftar_green; font-weight: bold; }\n\
         .daftar-removed-count { color: @daftar_red; font-weight: bold; }\n\
         .daftar-capture { padding: 16px; }\n\
         .daftar-capture .capture-icon { color: @daftar_accent; }\n\
         .daftar-capture .capture-field { font-size: 16pt; }\n\
         .daftar-capture .capture-error { color: @daftar_red; }\n\
         .daftar-crumbs .daftar-crumb { padding: 0 5px; min-height: 22px; }\n\
         .daftar-crumbs .daftar-crumb-note { color: @daftar_accent; }\n\
         .daftar-drop { background-color: alpha(@daftar_accent, 0.2); border-radius: 6px; }\n\
         .daftar-vim-float { border-radius: 6px; box-shadow: 0 4px 16px alpha(black, 0.25); }\n\
         .daftar-vim-float > label { border-radius: 6px; }\n",
    );
    out
}

/// libadwaita's own colors from the palette, so the header bars, the sidebar,
/// selections, switches and suggested buttons are naz's (or Daylight's) and
/// not Adwaita's blue and grey. libadwaita 1.6 and later read the variables;
/// the named colors are for older ones.
fn adwaita(palette: &[(&str, &str)]) -> String {
    let color = |name: &str| palette.iter().find(|(key, _)| *key == name).map_or("", |(_, color)| *color);
    let (background, panel, text, accent) = (color("background"), color("panel"), color("text"), color("accent"));
    let colors = [
        ("accent-bg-color", accent),
        ("accent-fg-color", color("on_accent")),
        ("accent-color", accent),
        ("destructive-bg-color", color("red")),
        ("destructive-color", color("red")),
        ("success-bg-color", color("green")),
        ("success-color", color("green")),
        ("warning-color", color("yellow")),
        ("error-color", color("red")),
        ("window-bg-color", background),
        ("window-fg-color", text),
        ("view-bg-color", background),
        ("view-fg-color", text),
        ("headerbar-bg-color", panel),
        ("headerbar-fg-color", text),
        ("headerbar-backdrop-color", background),
        ("sidebar-bg-color", panel),
        ("sidebar-fg-color", text),
        ("sidebar-backdrop-color", background),
        ("secondary-sidebar-bg-color", panel),
        ("secondary-sidebar-fg-color", text),
        ("secondary-sidebar-backdrop-color", background),
        ("popover-bg-color", panel),
        ("popover-fg-color", text),
        ("dialog-bg-color", panel),
        ("dialog-fg-color", text),
        ("thumbnail-bg-color", panel),
        ("thumbnail-fg-color", text),
    ];
    let mut out = String::from(":root {\n");
    for (name, value) in colors {
        out.push_str(&format!("  --{name}: {value};\n"));
    }
    out.push_str("}\n");
    for (name, value) in colors {
        out.push_str(&format!("@define-color {} {value};\n", name.replace('-', "_")));
    }
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

    #[test]
    fn adwaita_takes_the_palette_and_not_its_blue() {
        let naz = css(&NAZ);
        assert!(naz.contains("--accent-bg-color: #ffb040;"));
        assert!(naz.contains("--accent-fg-color: #323232;"));
        assert!(naz.contains("--window-bg-color: #323232;"));
        assert!(naz.contains("--sidebar-bg-color: #3a3a35;"));
        assert!(naz.contains("@define-color accent_bg_color #ffb040;"));
        let daylight = css(&DAYLIGHT);
        assert!(daylight.contains("--accent-bg-color: #a85200;"));
        assert!(daylight.contains("--accent-fg-color: #ffffff;"));
    }
}
