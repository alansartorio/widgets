//! What the kimai popup adds to the house style.

use widgets_gtkui::Keyboard;

/// Anchor the window under the bar.
///
/// `OnDemand` for the keyboard: this window only wants Escape, but it does want
/// it — a layer surface has no titlebar to close from, so without a keyboard
/// mode the only way out is the ✕ or another right click on the pill.
pub fn place(win: &relm4::gtk::Window) {
    widgets_gtkui::place(win, Keyboard::OnDemand);
}

pub fn load_css() {
    widgets_gtkui::load_css(&format!("{}{KIMAI}", widgets_gtkui::css::shell()));
}

const KIMAI: &str = r#"
    /* Wide enough that "0s / 8:00:00" and "142:00:00 / 160:00:00" produce the
       same shaped window, instead of it growing under the pointer as the month
       fills up. */
    .widgets-popup { min-width: 320px; }

    .kimai-running {
        background-color: #434c5e;   /* nord2 */
        border-radius: 8px;
        padding: 8px 10px;
        margin-bottom: 10px;
    }
    .kimai-task { font-weight: bold; color: #eceff4; }
    .kimai-sub { font-size: 11px; color: alpha(#d8dee9, 0.6); }
    .kimai-elapsed {
        font-family: "RobotoMono Nerd Font", monospace;
        font-size: 15px;
        color: #eceff4;
    }

    /* One row per span. The label column is fixed-width so the three totals
       line up as a column rather than wandering with the length of "semana". */
    .kimai-row { margin-bottom: 8px; }
    .kimai-span { color: alpha(#d8dee9, 0.7); font-size: 12px; }
    .kimai-total {
        font-family: "RobotoMono Nerd Font", monospace;
        color: #eceff4;
    }
    .kimai-expected { color: alpha(#d8dee9, 0.5); font-size: 12px; }

    /* The whole point of the window. Ahead is nord14, behind is nord13 --
       amber rather than red, because being behind at 09:20 on a Monday is the
       normal state of affairs and should not read as an error. Red is reserved
       for the server being gone. */
    .kimai-delta {
        font-family: "RobotoMono Nerd Font", monospace;
        font-weight: bold;
    }
    .kimai-delta.ahead { color: #a3be8c; }    /* nord14 */
    .kimai-delta.behind { color: #ebcb8b; }   /* nord13 */

    /* The bar under each row. GTK draws a progressbar as a trough with a filled
       block inside it, and both need naming: the theme's own colours are for a
       light background and vanish against nord1. */
    progressbar.kimai-bar { min-height: 4px; }
    progressbar.kimai-bar trough {
        background-color: #434c5e;   /* nord2 */
        border: none;
        min-height: 4px;
    }
    progressbar.kimai-bar progress {
        background-color: #5e81ac;   /* nord10 */
        border: none;
        min-height: 4px;
    }
    progressbar.kimai-bar.holiday progress { background-color: #88c0d0; }  /* nord8 */

    .kimai-foot {
        margin-top: 4px;
        font-size: 11px;
        color: alpha(#d8dee9, 0.5);
    }
    .kimai-foot.holiday { color: #88c0d0; }   /* nord8 */
    .kimai-foot.warn { color: #ebcb8b; }      /* nord13 */
"#;
