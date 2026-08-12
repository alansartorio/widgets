//! The seven-by-seven grid of labels that is the calendar.
//!
//! Built once and refilled, rather than torn down and rebuilt on every month:
//! forty-nine widgets is not much, but replacing them all would make the window
//! flicker on every ‹ and lose whatever the pointer was hovering.

use chrono::Datelike;
use holidays::MonthView;
use relm4::gtk;
use relm4::gtk::prelude::*;

/// Columns, and also the number of weekday headings.
pub const COLS: i32 = 7;
/// Week rows under the headings. Always six; see `holidays::month::CELLS` for
/// why a five-row month still gets six.
pub const ROWS: i32 = 6;

pub struct Grid {
    pub widget: gtk::Grid,
    headings: Vec<gtk::Label>,
    days: Vec<gtk::Label>,
}

impl Grid {
    pub fn new() -> Self {
        let widget = gtk::Grid::new();
        widget.add_css_class("cal-grid");
        widget.set_row_homogeneous(true);
        widget.set_column_homogeneous(true);
        widget.set_row_spacing(2);
        widget.set_column_spacing(2);

        let headings: Vec<_> = (0..COLS)
            .map(|col| {
                let label = gtk::Label::new(None);
                label.add_css_class("cal-dow");
                widget.attach(&label, col, 0, 1, 1);
                label
            })
            .collect();

        let days: Vec<_> = (0..ROWS * COLS)
            .map(|i| {
                let label = gtk::Label::new(None);
                label.add_css_class("cal-day");
                // Row 0 is the headings, so the weeks start at 1.
                widget.attach(&label, i % COLS, i / COLS + 1, 1, 1);
                label
            })
            .collect();

        Self {
            widget,
            headings,
            days,
        }
    }

    /// Repaint from a month.
    pub fn show(&self, view: &MonthView) {
        for (label, text) in self.headings.iter().zip(view.weekday_labels) {
            label.set_label(text);
        }

        for (label, cell) in self.days.iter().zip(&view.cells) {
            label.set_label(&cell.day.date.day().to_string());

            // *Replaced*, not added. Adding would leave a stale `.holiday` on
            // the cell the first time you page past a feriado, and the grid
            // would slowly fill with red as you scrolled through the year.
            let mut classes = vec!["cal-day"];
            classes.extend(cell.classes());
            label.set_css_classes(&classes);

            label.set_tooltip_text(cell.tooltip().as_deref());
        }
    }
}
