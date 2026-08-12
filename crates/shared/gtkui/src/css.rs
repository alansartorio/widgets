//! The Nord palette and the CSS every popup starts from.
//!
//! These are the same sixteen colours the eww bar is painted in (its
//! `nord.scss`), repeated here rather than read from it: the bar's stylesheet
//! is a build artefact of a different repo, and a popup that silently loses its
//! colours because a file moved is worse than two lists that have not changed
//! since 2016.

/// Polar Night — backgrounds, and text on a light ambiance.
pub const NORD0: &str = "#2e3440";
pub const NORD1: &str = "#3b4252";
pub const NORD2: &str = "#434c5e";
pub const NORD3: &str = "#4c566a";
/// Snow Storm — text on a dark ambiance.
pub const NORD4: &str = "#d8dee9";
pub const NORD5: &str = "#e5e9f0";
pub const NORD6: &str = "#eceff4";
/// Frost — primary UI elements.
pub const NORD7: &str = "#8fbcbb";
pub const NORD8: &str = "#88c0d0";
pub const NORD9: &str = "#81a1c1";
pub const NORD10: &str = "#5e81ac";
/// Aurora — errors, annotations, warnings, success, numbers.
pub const NORD11: &str = "#bf616a";
pub const NORD12: &str = "#d08770";
pub const NORD13: &str = "#ebcb8b";
pub const NORD14: &str = "#a3be8c";
pub const NORD15: &str = "#b48ead";

/// The rules every popup in this workspace needs before it adds its own.
///
/// Both of them exist because GTK's default theme actively fights a popup that
/// wants to paint itself, and both were learned the hard way:
///
/// * The rounded popup is a box *inside* the window, and the window paints its
///   own themed background behind it — which shows through as four opaque
///   slivers where the border radius cuts the corners. So the window itself
///   must draw nothing and let the corners fall through to whatever is behind
///   the surface. Same one level in for scrolledwindow, viewport and stack,
///   whose theme backgrounds paint over the popup's colour in a slightly
///   different shade and show up as a seam.
///
/// * Adwaita paints buttons and entries with a `background-image`, which sits
///   on top of any `background-color` a stylesheet sets and leaves the widget
///   whatever colour the ambient GTK theme picked — white glyphs on a white
///   pill under the default light theme. Opting out has to name the image, the
///   border and the shadow, not just the colour.
pub const BASE: &str = r#"
    * { font-family: "DejaVu Sans", "RobotoMono Nerd Font", sans-serif; }

    window, window.background {
        background-color: transparent;
        background-image: none;
        box-shadow: none;
    }
    scrolledwindow, viewport, stack {
        background-color: transparent;
        background-image: none;
    }
    button, entry {
        background-image: none;
        border: none;
        box-shadow: none;
    }
"#;

/// The popup shell: the box that actually paints, and its title row.
///
/// Every window here is `.widgets-popup` with the app's own class beside it, so
/// the geometry is written once and only the contents differ.
pub const POPUP: &str = r#"
    .widgets-popup {
        background-color: #3b4252;   /* nord1 */
        border-radius: 12px;
        padding: 12px 14px;
        color: #eceff4;              /* nord6 */
    }
    .widgets-title { font-weight: bold; color: #eceff4; margin-left: 2px; }
    .widgets-head { margin-bottom: 10px; }
    .widgets-dim { color: alpha(#d8dee9, 0.6); }
    .widgets-error { color: #bf616a; font-size: 12px; margin-top: 6px; }

    .widgets-button {
        padding: 2px 10px;
        border-radius: 1000px;
        background-color: #434c5e;   /* nord2 */
        color: #eceff4;
    }
    .widgets-button:hover { background-color: #5e81ac; }        /* nord10 */
    .widgets-button.primary { background-color: #5e81ac; }
    .widgets-button.primary:hover { background-color: #81a1c1; } /* nord9 */
    .widgets-button.danger:hover { background-color: #bf616a; }  /* nord11 */
    .widgets-button:disabled {
        color: alpha(#d8dee9, 0.35);
        background-color: alpha(#434c5e, 0.5);
    }

    /* Every rule above sets `box-shadow: none` to stop Adwaita painting its own
       button chrome — which also removes the only thing GTK draws to show where
       keyboard focus is. On anything you are meant to Tab through, that is not
       a cosmetic loss: nothing on screen tells you where the next keystroke
       goes. So put an explicit ring back.

       On :focus rather than :focus-visible, which would be the tidier choice —
       it shows only for keyboard navigation and not after a click. But whether
       GTK considers a focus change "visible" is its own heuristic, and a ring
       that is sometimes absent defeats the point of having one. */
    .widgets-button:focus, entry:focus, entry:focus-within {
        outline: 2px solid #88c0d0;   /* nord8 */
        outline-offset: 2px;
    }
"#;

/// `BASE` and `POPUP` together — what a window that wants the house style
/// passes to [`load_css`](super::load_css) before appending its own rules.
pub fn shell() -> String {
    format!("{BASE}{POPUP}")
}
