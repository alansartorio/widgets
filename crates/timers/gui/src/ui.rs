//! What the timers windows add to the house style.
//!
//! Placement, the single-instance behaviour, the Nord palette and the popup
//! shell all live in `widgets_gtkui` now — including the reasoning that used to
//! be here about transparent windows and Adwaita's `background-image`, which
//! applies to every popup in the workspace rather than only these two. What is
//! left below is the parts of the list and the add sheet that no other window
//! has: the timer rows, the duration/time toggle and the form's fields.

pub use widgets_gtkui::{Keyboard, place};

/// Install the shared shell plus the rules below, once per process.
pub fn load_css() {
    widgets_gtkui::load_css(&format!("{}{TIMERS}", widgets_gtkui::css::shell()));
}

const TIMERS: &str = r#"
    /* Wide enough that a one-word timer and a long one produce the same shaped
       window, instead of the popup resizing under the pointer every time the
       list changes. */
    .widgets-popup { min-width: 300px; }

    /* The toggles that pick between a duration and a time of day. Checked has
       to be louder than hover, or the pressed one looks unselected the moment
       the pointer moves over the other. */
    .timers-toggle {
        padding: 2px 14px;
        background-color: #434c5e;   /* nord2 */
        color: alpha(#d8dee9, 0.7);
    }
    .timers-toggle:checked {
        background-color: #5e81ac;   /* nord10 */
        color: #eceff4;
        font-weight: bold;
    }
    .timers-toggle:first-child { border-radius: 1000px 0 0 1000px; }
    .timers-toggle:last-child { border-radius: 0 1000px 1000px 0; }
    .timers-toggle:focus { outline: 2px solid #88c0d0; outline-offset: 2px; }

    entry.timers-entry {
        background-color: #434c5e;   /* nord2 */
        color: #eceff4;
        border-radius: 8px;
        padding: 6px 10px;
        min-width: 220px;
    }
    entry.timers-entry:focus-within { background-color: #4c566a; }  /* nord3 */

    .timers-hint { font-size: 11px; color: alpha(#d8dee9, 0.5); margin: 4px 0 0 2px; }

    /* The hide-name checkbox. GTK draws the box itself on a `check` child node,
       and Adwaita paints that with its own background-image and border — the
       same thing the shared sheet opts buttons and entries out of, one level
       deeper and on a node it does not name. */
    checkbutton.timers-check { color: alpha(#d8dee9, 0.8); font-size: 12px; }
    checkbutton.timers-check check {
        background-image: none;
        box-shadow: none;
        border: 1px solid #4c566a;    /* nord3 */
        background-color: #434c5e;    /* nord2 */
        border-radius: 4px;
        min-width: 14px;
        min-height: 14px;
    }
    checkbutton.timers-check check:checked {
        background-color: #5e81ac;    /* nord10 */
        border-color: #5e81ac;
        color: #eceff4;
    }
    checkbutton.timers-check:focus { outline: 2px solid #88c0d0; outline-offset: 2px; }

    .timer-row {
        background-color: #434c5e;
        border-radius: 8px;
        padding: 8px 10px;
        margin-bottom: 6px;
    }
    /* An unacknowledged alarm is the one thing in this window that has to be
       visible from across the room. */
    .timer-row.firing {
        background-color: #4b3a40;
        border-left: 3px solid #bf616a;   /* nord11 */
    }
    .timer-name { font-weight: bold; color: #eceff4; }
    .timer-sub { font-size: 11px; color: alpha(#d8dee9, 0.6); }
    .timer-remaining {
        font-family: "RobotoMono Nerd Font", monospace;
        font-size: 15px;
        margin: 0 10px;
        color: #eceff4;
    }
    .timer-row.firing .timer-remaining { color: #bf616a; font-weight: bold; }

    .widgets-dim { padding: 10px 2px; }
"#;
