//! What the claude-usage popup adds to the house style.
//!
//! Placement, the single-instance behaviour, the Nord palette and the popup
//! shell come from `widgets_gtkui`. Only the chart's own spacing is left.

use widgets_gtkui::Keyboard;

/// Anchor the window under the bar.
///
/// `OnDemand` for the keyboard, which is not the default and matters: this
/// window only wants Escape, but it does want it — the popup has no titlebar to
/// close from, so without a keyboard mode the only way out is the ✕ or another
/// click on the bar.
pub fn place(win: &relm4::gtk::Window) {
    widgets_gtkui::place(win, Keyboard::OnDemand);
}

/// Install the shared shell plus the rules below, once per process.
pub fn load_css() {
    widgets_gtkui::load_css(&format!("{}{USAGE}", widgets_gtkui::css::shell()));
}

const USAGE: &str = r#"
    /* The view buttons sit in a row and want breathing room the generic pill
       does not give them. */
    .widgets-head .widgets-button { margin-right: 6px; }
"#;
