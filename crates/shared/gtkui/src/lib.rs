//! The chrome every popup in this workspace wears: layer-shell placement,
//! single-instance toggling, and the Nord palette the eww bar is painted in.
//!
//! All four GUIs are the same animal — a small window the bar spawns on a
//! click, anchored under the bar, dismissed by clicking again. Before this
//! crate each one carried its own copy of that, which is how timers ended up
//! missing [`single_instance`]'s second hook and leaking a process on every
//! dismissal.

pub mod css;

use std::cell::Cell;

use relm4::RelmApp;
use relm4::gtk;
use relm4::gtk::prelude::*;

/// How much of the keyboard a window asks the compositor for.
///
/// This is not optional detail: a layer surface gets `KeyboardMode::None` by
/// default, and none means *no key events ever arrive*. GTK still moves its own
/// focus ring around inside the window, so an entry looks focused when clicked
/// and simply swallows nothing — every keystroke goes to whatever the
/// compositor still considers focused, which is the window that was there
/// before. That is the whole bug behind "I can click the field but I cannot
/// type in it".
pub enum Keyboard {
    /// Focus when the user interacts, like a normal window. Right for a window
    /// driven by clicks that only wants Escape and the odd arrow key.
    ///
    /// The caveat worth a comment at every call site: keys arrive only *after*
    /// the window has been clicked. Arrow-key navigation that does nothing
    /// until you click first is this, not a broken handler.
    OnDemand,
    /// Take the keyboard for as long as the window is mapped. Right for a form:
    /// it exists to be typed into, and you should be able to start typing the
    /// moment it opens without clicking it first.
    Exclusive,
}

/// Anchor a window under the bar, WM-agnostically, via the layer-shell
/// protocol: top edge, overlay layer, 40px down to clear the bar itself. Left
/// and right both unanchored, so the surface takes its own width and the
/// compositor centres it.
///
/// Call this from a component's `init`, which is the last moment before the
/// window is shown — layer-shell has to be configured on an unrealized window.
pub fn place(win: &gtk::Window, keyboard: Keyboard) {
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    win.init_layer_shell();
    win.set_layer(Layer::Overlay);
    win.set_anchor(Edge::Top, true);
    win.set_margin(Edge::Top, 40);
    win.set_keyboard_mode(match keyboard {
        Keyboard::OnDemand => KeyboardMode::OnDemand,
        Keyboard::Exclusive => KeyboardMode::Exclusive,
    });
}

/// Build the application and make a second launch dismiss the window.
///
/// `visible_on_activate(false)` because Relm4's own activate handler re-shows
/// the window; with it off that handler does nothing and the toggle below is
/// the only one that matters. The component makes its root visible once it has
/// configured its layer surface.
///
/// `with_args` empty because nothing of ours should reach GTK: a binary here
/// may take an argument of its own (`timers-gui add`, `calendar-gui 2026-08`),
/// and GTK would reject it as an unknown option.
pub fn single_instance<M: std::fmt::Debug>(app_id: &str) -> RelmApp<M> {
    let app = RelmApp::new(app_id)
        .visible_on_activate(false)
        .with_args(Vec::new());
    toggle_on_relaunch();
    leave_when_another_instance_answers();
    app
}

/// Make a second launch dismiss the window instead of opening another.
///
/// The bar runs these binaries on every click, and the application id makes
/// them single-instance: the second launch never becomes a second process, it
/// hands another `activate` to the one already running. That would otherwise
/// stack a fresh popup on top of the old one. So the click that finds a window
/// open is the click that closes it — the popup toggles from the same button
/// that raised it, and any ✕ in its header stays for dismissing it without
/// going back to the bar.
///
/// Closing the last window drops the application's last hold, so the process
/// exits and the next click starts clean.
///
/// The first `activate` has to be skipped. Relm4 builds the component on
/// `startup`, which runs *before* the first `activate` — without the guard the
/// window would be built and then immediately closed by its own launch.
fn toggle_on_relaunch() {
    let first = Cell::new(true);
    relm4::main_application().connect_activate(move |app| {
        if first.replace(false) {
            return;
        }
        for window in app.windows() {
            window.close();
        }
    });
}

/// Quit as soon as it turns out this process is the *second* instance.
///
/// `RelmApp::run` ends with a blocking main-context iteration, and in a second
/// instance that never returns: `GApplication::run` hands the click to the
/// instance that owns the window and comes straight back, after which nothing
/// else will ever happen in this process. Left alone it sits there forever, so
/// every click that dismissed a popup would leak a process. That is exactly
/// what timers-gui did before this crate existed.
///
/// Registering is what decides which instance this is, so the hook is the
/// `is-registered` notification rather than a timer — registration happens
/// inside `run`, and it iterates the main context while it waits on the bus, so
/// anything armed beforehand can be dispatched *before* the answer exists.
fn leave_when_another_instance_answers() {
    relm4::main_application().connect_is_registered_notify(|app| {
        // The same notification arrives on the way *out*, when the application
        // unregisters — and asking an unregistered application whether it is
        // remote is a GLib assertion failure, which would land in the journal
        // on every dismissal.
        if !app.is_registered() || !app.is_remote() {
            return;
        }
        // Not from here, though: the click still has to reach the instance that
        // owns the window, and `run` sends it just after registering. An idle
        // callback is dispatched in the first main-context turn after that —
        // the drain at the end of `run` — by which point the message is on its
        // way.
        relm4::gtk::glib::idle_add_local_once(|| std::process::exit(0));
    });
}

/// Install a stylesheet for this process.
///
/// Relm4 has `RelmApp::set_global_css`, but that has to run before `run()`, and
/// only the component knows which window it is dressing — so this stays a plain
/// provider installed from `init`. Calling it twice is harmless; only one
/// window exists per process anyway.
pub fn load_css(sheet: &str) {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(sheet);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
