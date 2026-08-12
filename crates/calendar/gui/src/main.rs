//! calendar-gui — the month the clock opens.
//!
//! Replaces eww's `defwindow calendar`, which wrapped GTK's own `calendar`
//! widget in a box and pushed today's date into three `defvar`s before every
//! open. Two reasons it had to go rather than be restyled:
//!
//! * `GtkCalendar` can only mark a day *binary* — a marked day gets the
//!   `:selected` pseudo-class and there is no third state — so a feriado and a
//!   día no laborable cannot be told apart, and a puente cannot be shown as its
//!   own thing at all.
//! * It knew nothing about any of them in the first place.
//!
//! So the grid is hand-built out of labels, each carrying its own CSS classes,
//! and the days come from the `holidays` crate — the same one the kimai pill
//! counts its month with, so the two cannot disagree about a Thursday.
//!
//! `calendar-gui [YYYY-MM]` opens on a given month; bare opens on this one.

mod grid;
mod month;
mod ui;

fn main() {
    // Parsed before GTK sees the arguments, because `single_instance` hands it
    // an empty argv — GTK would reject `2026-08` as an unknown option.
    let start = std::env::args().nth(1).and_then(|arg| month::parse(&arg));
    widgets_gtkui::single_instance("ar.com.alansartorio.calendar-gui").run::<month::Month>(start);
}
