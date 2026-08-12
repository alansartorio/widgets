//! claude-usage-gui — the usage chart, as a Relm4 component on a GTK4
//! layer-shell window.
//!
//! Replaces the eww `claude-usage-plot` window. Shows the chart the popup did,
//! plus one layer: 7-day windows at the back, 5-hour session rectangles over
//! them, hourly (day) or daily (month) bars in front, with day/month
//! navigation. Reads nothing from disk — it asks the daemon for chart data over
//! the same socket the CLI uses.
//!
//! Relm4 rather than gtk4-rs directly, so the window is a model, a message enum
//! and a `view!` tree instead of widget handles cloned into closures and shared
//! state behind `Rc<RefCell<_>>`. The two things the framework has no opinion
//! about — the toggle-on-second-launch behaviour and the layer-shell setup —
//! come from `widgets_gtkui`, shared with the other three popups.

mod plot;
mod popup;
mod ui;

fn main() {
    // Renamed from `com.example.claude-usage-gui` on the way into this repo, so
    // all four popups sit under one reverse-DNS prefix. The id is only ever
    // compared against itself — it is what makes the binary single-instance —
    // so changing it costs nothing beyond one stale process at the moment of
    // the switch.
    widgets_gtkui::single_instance("ar.com.alansartorio.claude-usage-gui").run::<popup::Popup>(());
}
