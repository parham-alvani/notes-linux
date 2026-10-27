//! Ctrl+Alt+G: the notes around the open one, or the whole vault, as a
//! graph, as on the Mac. Each top folder has its color, a node grows with its
//! links, the open note glows, and pointing at a note lights its links. Drag
//! to move, scroll to zoom, click a note to open it.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::f64::consts::TAU;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{GraphLayout, GraphNode, NoteGraph, Vault};

use crate::store::VaultStore;

/// The palette's colors a folder may take, dark then light, in the Mac's order.
const COLORS: [(u32, u32); 9] = [
    (0xFFB040, 0xA85200),
    (0x80E5FF, 0x00707A),
    (0xFF69B4, 0xB02A70),
    (0xF0A0F0, 0x8040A0),
    (0xA0FF40, 0x5A7A00),
    (0x40D0FF, 0x0070A0),
    (0x20FFA0, 0x0A7A48),
    (0xFFE040, 0x8A6A00),
    (0xFF7088, 0xC01030),
];
const ACCENT: (u32, u32) = (0xFFB040, 0xA85200);
const MUTED: (u32, u32) = (0xA8A8A0, 0x6A6A62);
const TEXT: (u32, u32) = (0xF5F5F0, 0x2A2A28);
const BACKGROUND: (u32, u32) = (0x323232, 0xFAF9F5);

/// A folder's color, by the Mac's hash of its name, so both apps agree.
pub fn color_index(group: &str) -> usize {
    let mut hash: u32 = 0;
    for c in group.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(c as u32);
    }
    (hash % COLORS.len() as u32) as usize
}

fn rgb(pair: (u32, u32), dark: bool) -> (f64, f64, f64) {
    let value = if dark { pair.0 } else { pair.1 };
    (((value >> 16) & 0xFF) as f64 / 255.0, ((value >> 8) & 0xFF) as f64 / 255.0, (value & 0xFF) as f64 / 255.0)
}

fn group_color(group: &str) -> (u32, u32) {
    if group.is_empty() { MUTED } else { COLORS[color_index(group)] }
}

fn radius(node: &GraphNode) -> f64 {
    (if node.distance == Some(0) { 9.0 } else { 3.5 }) + (node.degree as f64).sqrt() * 1.8
}

struct State {
    graph: NoteGraph,
    layout: Option<GraphLayout>,
    hovered: Option<usize>,
    zoom: f64,
    pan: (f64, f64),
    /// The pan when a drag began.
    dragged_from: (f64, f64),
    /// The pointer moved since the button went down: a drag, not a click.
    moved: bool,
}

/// Where the graph's coordinates fall in a view of [width] by [height].
fn transform(layout: &GraphLayout, zoom: f64, pan: (f64, f64), width: f64, height: f64) -> (f64, f64, f64) {
    let fold = |values: &[f64]| values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let ((min_x, max_x), (min_y, max_y)) = (fold(&layout.x), fold(&layout.y));
    if layout.x.is_empty() {
        return (1.0, width / 2.0, height / 2.0);
    }
    let fit = ((width - 80.0) / (max_x - min_x).max(1.0)).min((height - 80.0) / (max_y - min_y).max(1.0)).min(2.2);
    let scale = fit * zoom;
    (scale, width / 2.0 - (min_x + max_x) / 2.0 * scale + pan.0, height / 2.0 - (min_y + max_y) / 2.0 * scale + pan.1)
}

pub fn present(parent: &impl IsA<gtk::Widget>, store: Rc<VaultStore>, center: Option<String>, on_open: impl Fn(&str) + 'static) {
    let whole = Rc::new(Cell::new(center.is_none()));
    let depth = Rc::new(Cell::new(2usize));
    let state = Rc::new(RefCell::new(State {
        graph: NoteGraph { nodes: vec![], edges: vec![] },
        layout: None,
        hovered: None,
        zoom: 1.0,
        pan: (0.0, 0.0),
        dragged_from: (0.0, 0.0),
        moved: false,
    }));

    let title = adw::WindowTitle::new("Graph", "");
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    let scope = gtk::Box::builder().css_classes(["linked"]).build();
    let this_note = gtk::ToggleButton::builder().label("This note").active(center.is_some()).sensitive(center.is_some()).build();
    let vault = gtk::ToggleButton::builder().label("Whole vault").active(center.is_none()).group(&this_note).build();
    scope.append(&this_note);
    scope.append(&vault);
    header.pack_start(&scope);
    let steps = gtk::SpinButton::with_range(1.0, 4.0, 1.0);
    steps.set_value(2.0);
    steps.set_tooltip_text(Some("Steps away from the note"));
    steps.set_sensitive(center.is_some());
    header.pack_start(&steps);
    let fit = gtk::Button::builder().icon_name("zoom-fit-best-symbolic").tooltip_text("Fit the graph to the window").build();
    header.pack_end(&fit);

    let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build();
    let overlay = gtk::Overlay::builder().child(&area).build();
    overlay.add_overlay(&spinner);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    let legend = gtk::Box::builder().spacing(10).margin_start(12).margin_end(12).margin_top(8).margin_bottom(8).build();
    let legend_scroll = gtk::ScrolledWindow::builder().vscrollbar_policy(gtk::PolicyType::Never).child(&legend).build();

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&overlay));
    view.add_bottom_bar(&legend_scroll);
    let dialog = adw::Dialog::builder().content_width(980).content_height(720).child(&view).build();

    {
        let state = state.clone();
        area.set_draw_func(move |area, cr, width, height| draw(&state.borrow(), area, cr, width as f64, height as f64));
    }

    // Builds the graph for the scope, and lays it out away from the window.
    let build = {
        let (state, area, spinner, title, legend, store, whole, depth) =
            (state.clone(), area.clone(), spinner.clone(), title.clone(), legend.clone(), store.clone(), whole.clone(), depth.clone());
        let center = center.clone();
        Rc::new(move || {
            let graph = {
                let index = store.index();
                match center.as_deref().filter(|_| !whole.get()) {
                    Some(path) => NoteGraph::local(&index, path, depth.get(), Some(&store.excluded())),
                    None => NoteGraph::vault(&index, Some(&store.excluded())),
                }
            };
            let name = match center.as_deref().filter(|_| !whole.get()) {
                Some(path) => format!("Graph of {}", Vault::title(path)),
                None => format!("Graph of {}", store.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            };
            title.set_title(&name);
            title.set_subtitle(&format!("{} notes · {} links", graph.nodes.len(), graph.edges.len()));
            fill_legend(&legend, &graph);
            let pin = (!whole.get()).then_some(0);
            {
                let mut state = state.borrow_mut();
                state.layout = None;
                state.hovered = None;
                state.graph = graph.clone();
            }
            spinner.set_visible(true);
            area.queue_draw();
            let (state, area, spinner) = (state.clone(), area.clone(), spinner.clone());
            glib::spawn_future_local(async move {
                let count = graph.nodes.len();
                let Ok(layout) = gio::spawn_blocking(move || {
                    let mut layout = GraphLayout::new(&graph, pin);
                    layout.run(if count > 800 { 160 } else { 300 });
                    layout
                })
                .await
                else {
                    return;
                };
                let mut current = state.borrow_mut();
                if current.graph.nodes.len() == count {
                    current.layout = Some(layout);
                }
                drop(current);
                spinner.set_visible(false);
                area.queue_draw();
            });
        })
    };
    build();

    {
        let (whole, build) = (whole.clone(), build.clone());
        vault.connect_toggled(move |button| {
            if whole.get() != button.is_active() {
                whole.set(button.is_active());
                build();
            }
        });
    }
    {
        let (depth, build) = (depth.clone(), build.clone());
        steps.connect_value_changed(move |spin| {
            depth.set(spin.value() as usize);
            build();
        });
    }
    {
        let (state, area) = (state.clone(), area.clone());
        fit.connect_clicked(move |_| {
            let mut state = state.borrow_mut();
            state.zoom = 1.0;
            state.pan = (0.0, 0.0);
            area.queue_draw();
        });
    }

    let motion = gtk::EventControllerMotion::new();
    {
        let (state, area) = (state.clone(), area.clone());
        motion.connect_motion(move |_, x, y| {
            let hovered = nearest(&state.borrow(), &area, x, y);
            if state.borrow().hovered != hovered {
                state.borrow_mut().hovered = hovered;
                area.queue_draw();
            }
        });
    }
    area.add_controller(motion);

    let drag = gtk::GestureDrag::new();
    {
        let state = state.clone();
        drag.connect_drag_begin(move |_, _, _| {
            let mut state = state.borrow_mut();
            state.dragged_from = state.pan;
            state.moved = false;
        });
    }
    {
        let (state, area) = (state.clone(), area.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            let mut state = state.borrow_mut();
            state.moved |= dx.abs() + dy.abs() > 3.0;
            state.pan = (state.dragged_from.0 + dx, state.dragged_from.1 + dy);
            area.queue_draw();
        });
    }
    area.add_controller(drag);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    {
        let (state, area) = (state.clone(), area.clone());
        scroll.connect_scroll(move |_, _, dy| {
            let mut state = state.borrow_mut();
            state.zoom = (state.zoom * if dy < 0.0 { 1.15 } else { 1.0 / 1.15 }).clamp(0.2, 8.0);
            area.queue_draw();
            glib::Propagation::Stop
        });
    }
    area.add_controller(scroll);

    let click = gtk::GestureClick::new();
    {
        let (state, area, dialog) = (state.clone(), area.clone(), dialog.clone());
        let chosen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let chosen = chosen.clone();
            dialog.connect_closed(move |_| {
                if let Some(path) = chosen.take() {
                    on_open(&path);
                }
            });
        }
        click.connect_released(move |_, _, x, y| {
            let state = state.borrow();
            // A drag ends in a release too; only a click opens.
            if state.moved {
                return;
            }
            if let Some(at) = nearest(&state, &area, x, y) {
                *chosen.borrow_mut() = Some(state.graph.nodes[at].path.clone());
                dialog.close();
            }
        });
    }
    area.add_controller(click);

    dialog.present(Some(parent));
}

fn fill_legend(legend: &gtk::Box, graph: &NoteGraph) {
    while let Some(child) = legend.first_child() {
        legend.remove(&child);
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &graph.nodes {
        *counts.entry(node.group.as_str()).or_default() += 1;
    }
    let mut groups: Vec<(&str, usize)> = counts.into_iter().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    let dark = adw::StyleManager::default().is_dark();
    for (group, count) in groups.into_iter().take(8) {
        let (r, g, b) = rgb(group_color(group), dark);
        let dot = gtk::DrawingArea::builder().content_width(9).content_height(9).valign(gtk::Align::Center).build();
        dot.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(r, g, b);
            cr.arc(w as f64 / 2.0, h as f64 / 2.0, 4.5, 0.0, TAU);
            let _ = cr.fill();
        });
        let item = gtk::Box::builder().spacing(5).build();
        item.append(&dot);
        item.append(&gtk::Label::builder().label(if group.is_empty() { "Vault root" } else { group }).css_classes(["caption"]).build());
        item.append(&gtk::Label::builder().label(count.to_string()).css_classes(["caption", "dim-label", "numeric"]).build());
        legend.append(&item);
    }
}

fn nearest(state: &State, area: &gtk::DrawingArea, x: f64, y: f64) -> Option<usize> {
    let layout = state.layout.as_ref()?;
    let (scale, dx, dy) = transform(layout, state.zoom, state.pan, area.width() as f64, area.height() as f64);
    let mut best: Option<(usize, f64)> = None;
    for (i, node) in state.graph.nodes.iter().enumerate() {
        let (px, py) = (layout.x[i] * scale + dx - x, layout.y[i] * scale + dy - y);
        let distance = (px * px + py * py).sqrt();
        if distance < (radius(node) + 4.0).max(12.0) && best.is_none_or(|(_, d)| distance < d) {
            best = Some((i, distance));
        }
    }
    best.map(|(i, _)| i)
}

fn draw(state: &State, area: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: f64, height: f64) {
    let dark = adw::StyleManager::default().is_dark();
    let (r, g, b) = rgb(BACKGROUND, dark);
    cr.set_source_rgb(r, g, b);
    let _ = cr.paint();
    let Some(layout) = state.layout.as_ref() else { return };
    let (scale, dx, dy) = transform(layout, state.zoom, state.pan, width, height);
    let point = |i: usize| (layout.x[i] * scale + dx, layout.y[i] * scale + dy);
    let graph = &state.graph;
    let lit: HashSet<usize> = match state.hovered {
        Some(h) => graph
            .edges
            .iter()
            .filter_map(|e| {
                if e.from == h {
                    Some(e.to)
                } else if e.to == h {
                    Some(e.from)
                } else {
                    None
                }
            })
            .chain([h])
            .collect(),
        None => HashSet::new(),
    };

    // Links: faint, and lit in the accent when they touch the pointed note.
    for edge in &graph.edges {
        let on = state.hovered.is_some_and(|h| edge.from == h || edge.to == h);
        let (r, g, b) = rgb(if on { ACCENT } else { MUTED }, dark);
        cr.set_source_rgba(
            r,
            g,
            b,
            if on {
                0.9
            } else if state.hovered.is_none() {
                0.28
            } else {
                0.1
            },
        );
        cr.set_line_width(if on { 1.8 } else { 0.8 });
        let ((x1, y1), (x2, y2)) = (point(edge.from), point(edge.to));
        cr.move_to(x1, y1);
        cr.line_to(x2, y2);
        let _ = cr.stroke();
    }

    let show_all = graph.nodes.len() <= 120 || scale > 1.4;
    let pango = area.pango_context();
    for (i, node) in graph.nodes.iter().enumerate() {
        let (x, y) = point(i);
        let r = radius(node) * scale.clamp(0.6, 1.6);
        let dim = state.hovered.is_some() && !lit.contains(&i);
        let center = node.distance == Some(0);
        let (fr, fg, fb) = rgb(if center { ACCENT } else { group_color(&node.group) }, dark);
        if center || state.hovered == Some(i) {
            let glow = gtk::cairo::RadialGradient::new(x, y, r, x, y, r * 2.2);
            glow.add_color_stop_rgba(0.0, fr, fg, fb, 0.45);
            glow.add_color_stop_rgba(1.0, fr, fg, fb, 0.0);
            let _ = cr.set_source(&glow);
            cr.arc(x, y, r * 2.2, 0.0, TAU);
            let _ = cr.fill();
        }
        cr.set_source_rgba(fr, fg, fb, if dim { 0.25 } else { 1.0 });
        cr.arc(x, y, r, 0.0, TAU);
        let _ = cr.fill();
        if show_all || center || lit.contains(&i) || node.degree >= 12 {
            let text = gtk::pango::Layout::new(&pango);
            text.set_text(&node.title);
            let mut font = gtk::pango::FontDescription::new();
            font.set_size(((if center { 13.0 } else { 10.5 }) * gtk::pango::SCALE as f64) as i32);
            if center || state.hovered == Some(i) {
                font.set_weight(gtk::pango::Weight::Semibold);
            }
            text.set_font_description(Some(&font));
            let (w, _) = text.pixel_size();
            let (tr, tg, tb) = rgb(
                if dim {
                    MUTED
                } else if center {
                    ACCENT
                } else {
                    TEXT
                },
                dark,
            );
            cr.set_source_rgba(tr, tg, tb, if dim { 0.4 } else { 0.85 });
            cr.move_to(x - w as f64 / 2.0, y + r + 6.0);
            pangocairo::functions::show_layout(cr, &text);
        }
    }

    // The note under the pointer, in a corner.
    if let Some(node) = state.hovered.and_then(|h| graph.nodes.get(h)) {
        let text = gtk::pango::Layout::new(&pango);
        let links = if node.degree == 1 { "1 link".to_string() } else { format!("{} links", node.degree) };
        text.set_markup(&format!(
            "<b>{}</b>\n<small>{links} · {}</small>",
            glib::markup_escape_text(&node.title),
            glib::markup_escape_text(&node.path)
        ));
        text.set_alignment(gtk::pango::Alignment::Right);
        let (w, h) = text.pixel_size();
        let (tr, tg, tb) = rgb(TEXT, dark);
        cr.set_source_rgba(tr, tg, tb, 0.9);
        cr.move_to(width - w as f64 - 16.0, height - h as f64 - 12.0);
        pangocairo::functions::show_layout(cr, &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_has_the_macs_color() {
        // The Mac's hash: h = h * 31 + scalar, then the order of its palette.
        assert_eq!(color_index("a"), 97 % 9);
        assert_eq!(color_index(""), 0);
        assert_eq!(group_color(""), MUTED);
    }
}
