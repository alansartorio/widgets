//! The window: a month, its feriados named under it, and the arrows to move.

use chrono::{Datelike, Local};
use holidays::{Calendar, WorkWeek};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use crate::grid::Grid;
use crate::ui;

/// How stale the feriado cache may get before this window refreshes it.
///
/// A week. The data changes by decree, months ahead, so anything shorter is
/// asking a public API for an answer that has not moved.
const MAX_AGE_DAYS: i64 = 7;

pub struct Month {
    cal: Calendar,
    year: i32,
    month: u32,
    grid: Grid,
}

#[derive(Debug)]
pub enum MonthMsg {
    Prev,
    Next,
    Today,
    /// A background refresh landed; re-read the calendar off disk and repaint.
    Refreshed,
    Close,
}

/// `2026-08`, or `2026-8`. Anything else is ignored rather than fatal — a
/// mistyped argument should still open this month's calendar.
pub fn parse(raw: &str) -> Option<(i32, u32)> {
    let (year, month) = raw.split_once('-')?;
    let year = year.parse().ok()?;
    let month = month.parse().ok()?;
    (1..=12).contains(&month).then_some((year, month))
}

impl Month {
    fn view(&self) -> holidays::MonthView {
        self.cal.month_view(self.year, self.month)
    }

    fn repaint(&mut self) {
        self.cal.ensure_year(self.year);
        self.grid.show(&self.view());
    }

    fn step(&mut self, forward: bool) {
        let view = self.view();
        (self.year, self.month) = if forward { view.next() } else { view.prev() };
        self.repaint();
    }

    fn title(&self) -> String {
        self.view().title
    }

    /// The named non-working days this month, one per line.
    ///
    /// A colour says *that* a day is special; only the name says which, and a
    /// tooltip you have to go looking for is not an answer to "why is the 24th
    /// red".
    fn holiday_list(&self) -> String {
        let view = self.view();
        if view.degraded {
            return "sin datos de feriados para este año".into();
        }
        if view.holidays.is_empty() {
            return "sin feriados este mes".into();
        }
        view.holidays
            .iter()
            .map(|day| {
                format!(
                    "{:>2}  {}{}",
                    day.date.day(),
                    day.name.as_deref().unwrap_or("(sin nombre)"),
                    if day.overridden { "  ·  a mano" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn degraded(&self) -> bool {
        self.view().degraded
    }
}

#[relm4::component(pub)]
impl Component for Month {
    /// The month to open on, if one was given on the command line.
    type Init = Option<(i32, u32)>;
    type Input = MonthMsg;
    type Output = ();
    type CommandOutput = ();

    view! {
        gtk::Window {
            set_title: Some("calendario"),

            // Escape closes; the arrows page. Note the OnDemand caveat: keys
            // only arrive once the window has been clicked, so an arrow press
            // that "does nothing" is the compositor still pointing at whatever
            // was focused before, not a dead handler.
            add_controller = gtk::EventControllerKey {
                connect_key_pressed[sender] => move |_, key, _, _| {
                    use gtk::gdk::Key;
                    match key {
                        Key::Escape => sender.input(MonthMsg::Close),
                        Key::Left | Key::h => sender.input(MonthMsg::Prev),
                        Key::Right | Key::l => sender.input(MonthMsg::Next),
                        Key::t | Key::Home => sender.input(MonthMsg::Today),
                        _ => {}
                    }
                    gtk::glib::Propagation::Proceed
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                add_css_class: "widgets-popup",
                add_css_class: "cal-popup",

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    add_css_class: "widgets-head",
                    set_spacing: 4,

                    gtk::Button {
                        set_label: "‹",
                        add_css_class: "widgets-button",
                        connect_clicked => MonthMsg::Prev,
                    },
                    gtk::Button {
                        set_label: "›",
                        add_css_class: "widgets-button",
                        connect_clicked => MonthMsg::Next,
                    },
                    gtk::Label {
                        add_css_class: "widgets-title",
                        add_css_class: "cal-title",
                        set_margin_start: 4,
                        set_hexpand: true,
                        set_xalign: 0.0,
                        #[watch]
                        set_label: &model.title(),
                    },
                    gtk::Button {
                        set_label: "hoy",
                        add_css_class: "widgets-button",
                        connect_clicked => MonthMsg::Today,
                    },
                    gtk::Button {
                        set_label: "✕",
                        add_css_class: "widgets-button",
                        connect_clicked => MonthMsg::Close,
                    },
                },

                #[local_ref]
                grid_widget -> gtk::Grid {},

                gtk::Label {
                    add_css_class: "cal-holidays",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 10,
                    #[watch]
                    set_label: &model.holiday_list(),
                    #[watch]
                    set_css_classes: if model.degraded() {
                        &["cal-holidays", "degraded"]
                    } else {
                        &["cal-holidays"]
                    },
                },
            }
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        ui::place(&root);
        ui::load_css();

        let today = Local::now().date_naive();
        let (year, month) = init.unwrap_or((today.year(), today.month()));

        let mut model = Month {
            // The work week only decides which column the grid starts on and
            // which days read as weekend; this window shows no hours, so the
            // default is right for it. kimai's own config is what the *pill*
            // counts with.
            cal: Calendar::open(WorkWeek::default()),
            year,
            month,
            grid: Grid::new(),
        };
        model.repaint();

        // Paint from the cache first and refresh behind it. A calendar that
        // waits on a public API before drawing August is a calendar that hangs
        // when the wifi is bad, and the answer it is waiting for changes about
        // three times a year.
        std::thread::spawn({
            let sender = sender.clone();
            move || {
                let mut cal = Calendar::open(WorkWeek::default());
                if cal.refresh_if_stale(chrono::Duration::days(MAX_AGE_DAYS))
                    == holidays::Refresh::Updated
                {
                    sender.input(MonthMsg::Refreshed);
                }
            }
        });

        let grid_widget = &model.grid.widget.clone();
        let widgets = view_output!();
        root.set_visible(true);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            MonthMsg::Close => root.close(),
            MonthMsg::Prev => self.step(false),
            MonthMsg::Next => self.step(true),
            MonthMsg::Today => {
                let today = Local::now().date_naive();
                self.year = today.year();
                self.month = today.month();
                self.repaint();
            }
            MonthMsg::Refreshed => {
                // The refresh happened in another `Calendar` — it had to, to
                // stay off this thread — so this one re-reads what it wrote.
                self.cal = Calendar::open(WorkWeek::default());
                self.repaint();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn a_month_argument_takes_both_spellings() {
        assert_eq!(parse("2026-08"), Some((2026, 8)));
        assert_eq!(parse("2026-8"), Some((2026, 8)));
        assert_eq!(parse("2026-13"), None);
        assert_eq!(parse("agosto"), None);
        assert_eq!(parse(""), None);
    }
}
