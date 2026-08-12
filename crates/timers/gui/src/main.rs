//! timers-gui — the timer list and the add sheet, as Relm4 components on
//! GTK4 layer-shell windows.
//!
//! Two modes in one binary, picked from argv: bare `timers-gui` shows the list
//! the bar's left click opens, `timers-gui add` shows the small form its right
//! click opens. They get separate application ids so each toggles on its own —
//! clicking the bar twice dismisses the list without disturbing an add sheet
//! that happens to be open, and vice versa.
//!
//! Reads nothing from disk: it asks the daemon over the same socket the CLI
//! uses, and every value it shows was computed there.
//!
//! Relm4 rather than gtk4-rs directly, so each window is a model, a message
//! enum and a `view!` tree instead of a pile of widget handles threaded through
//! closures. The two things Relm4 has no opinion about — the single-instance
//! toggling and the layer-shell setup — come from `widgets_gtkui`, which is
//! also where the fix for this binary leaking a process per dismissal lives.

mod add;
mod list;
mod row;
mod ui;

fn main() {
    let add_mode = matches!(
        std::env::args().nth(1).as_deref(),
        Some("add") | Some("new")
    );

    // Distinct ids per mode, so the two windows toggle independently. A single
    // id would make the add sheet's launch dismiss an open list.
    let app_id = if add_mode {
        "ar.com.alansartorio.timers-gui.add"
    } else {
        "ar.com.alansartorio.timers-gui"
    };

    // Built per branch rather than once: `RelmApp` is generic over the
    // top-level component's message type, so one binding cannot serve both.
    if add_mode {
        widgets_gtkui::single_instance(app_id).run::<add::Add>(());
    } else {
        widgets_gtkui::single_instance(app_id).run::<list::List>(());
    }
}
