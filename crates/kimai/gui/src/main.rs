//! kimai-gui — the three numbers the pill has no room for.
//!
//! The bar's pill shows today and, in eight characters, how the week stands.
//! This is where that gets said properly: hoy, semana and mes, each against
//! what it was *supposed* to be, with the difference spelled out. Read-only,
//! like the pill — starting and stopping clocks is Kimai's own page's job, and
//! the middle click still goes there.
//!
//! Opened by a right click on the pill and dismissed by another one, which is
//! `widgets_gtkui::single_instance` doing the work.
//!
//! No daemon behind it, unlike claude-usage-gui and timers-gui: this builds its
//! own `Poller` and drives it while the window is open, then exits with the
//! window. What it does borrow is the last line `kimai tail` wrote, so the
//! window has numbers in it from the first frame rather than after its own
//! first request comes back.

mod popup;
mod ui;

fn main() {
    widgets_gtkui::single_instance("ar.com.alansartorio.kimai-gui").run::<popup::Popup>(());
}
