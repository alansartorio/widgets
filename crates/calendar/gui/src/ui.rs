//! What the calendar adds to the house style.
//!
//! Ported from the eww `.cal-*` rules with `nord.scss` resolved, so the window
//! that replaces `defwindow calendar` looks like the one it replaces — plus the
//! four states GTK's own calendar widget had no way to show.

use widgets_gtkui::Keyboard;

/// Anchor the window under the bar.
///
/// `OnDemand`: this is a read-and-click window, and taking the keyboard away
/// from whatever you were typing in to show a calendar would be rude. It does
/// want Escape and the arrows, which `None` would deny it entirely.
pub fn place(win: &relm4::gtk::Window) {
    widgets_gtkui::place(win, Keyboard::OnDemand);
}

pub fn load_css() {
    widgets_gtkui::load_css(&format!("{}{CAL}", widgets_gtkui::css::shell()));
}

const CAL: &str = r#"
    /* eww's .cal-inner-box padding, kept: the grid needs air around it or the
       first column sits against the popup's edge. */
    .cal-popup { padding: 14px 14px 10px; }
    .cal-title { color: #5e81ac; }        /* nord10, as eww's .cal.header */

    .cal-dow {
        color: alpha(#d8dee9, 0.55);
        font-size: 11px;
        padding-bottom: 4px;
    }

    .cal-day {
        color: #d8dee9;                   /* nord4 */
        min-width: 30px;
        min-height: 30px;
        border-radius: 1000px;
    }

    /* eww's `.cal:indeterminate` — the days borrowed from the months either
       side. Dim enough to read as scaffolding, not so dim as to be invisible:
       "is the 1st a Monday" is a question you answer off these. */
    .cal-day.other-month { color: alpha(#d8dee9, 0.25); }

    /* eww's `.cal:selected`: a white pill with dark text. */
    .cal-day.today {
        background-color: #eceff4;        /* nord6 */
        color: #2e3440;                   /* nord0 */
        font-weight: bold;
    }

    .cal-day.weekend { color: alpha(#d8dee9, 0.45); }

    /* The three states GtkCalendar could not tell apart, which is why this
       window exists. Aurora, in descending order of "you definitely are not
       working": feriado, día no laborable, puente. */
    .cal-day.holiday { color: #bf616a; font-weight: bold; }   /* nord11 */
    .cal-day.non-working { color: #ebcb8b; }                  /* nord13 */
    .cal-day.bridge { color: #d08770; }                       /* nord12 */

    /* Today wins over all of them: a feriado you are looking at from itself is
       still the day you want to find first. */
    .cal-day.today.holiday,
    .cal-day.today.non-working,
    .cal-day.today.bridge {
        background-color: #eceff4;
        color: #2e3440;
    }

    .cal-holidays {
        font-size: 11px;
        color: alpha(#d8dee9, 0.65);
        border-top: 1px solid alpha(#d8dee9, 0.12);
        padding-top: 8px;
    }
    .cal-holidays.degraded { color: #ebcb8b; }   /* nord13 */
"#;
