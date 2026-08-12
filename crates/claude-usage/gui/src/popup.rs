//! The popup: a header that navigates and a chart that draws whatever view the
//! header landed on.
//!
//! The whole window is a function of two values — which [`View`] is showing, and
//! the [`Chart`] the daemon answered with for it. Every button does one thing:
//! sets the view and re-asks. Nothing here reads a widget back to find out what
//! the window is currently showing.

use std::rc::Rc;

use chrono::{Duration, NaiveDate};
use claude_usage_core::client;
use claude_usage_core::models::{Chart, View};
use claude_usage_core::paths::{ClaudeUsagePaths, Paths};
use claude_usage_core::proto::{Request, Response};
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::plot::{self, CHART_HEIGHT, LABEL_HEIGHT};
use crate::ui;

pub struct Popup {
    paths: Paths,
    /// What is being asked for. The chart below is the answer to *this*, so the
    /// two are only ever set together — see [`Popup::show`].
    view: View,
    /// `None` when the daemon did not answer, which is a different state from an
    /// empty chart (a day with no usage recorded) and reads differently in the
    /// header.
    ///
    /// Behind an `Rc` because the draw closure needs a copy of it on every render
    /// pass, and a chart is a few hundred strings.
    chart: Option<Rc<Chart>>,
    /// Which bar the pointer is over. Only the month view does anything with it,
    /// but the pointer doesn't know that, so the state is kept either way.
    hover: Option<usize>,
}

#[derive(Debug)]
pub enum PopupMsg {
    /// Zoom out from a day to the month it's in.
    Month,
    /// Step this many days from the day showing. Only meaningful in the day view,
    /// and the buttons that send it are insensitive in the other one.
    Step(i64),
    /// A click landed at this x. In the month view that drills into the day under
    /// it; anywhere else it means nothing, since a day's bars are hours already
    /// on screen.
    Drill(f64),
    /// The pointer moved to this x, or left the chart entirely.
    Hover(Option<f64>),
    Close,
}

impl Popup {
    fn chart(&self) -> Option<&Chart> {
        self.chart.as_deref()
    }

    /// The header's title, or what to say instead when there was no answer.
    fn title(&self) -> &str {
        self.chart()
            .map_or("no answer", |chart| chart.title.as_str())
    }

    /// Whether a day is showing. Navigation only makes sense from one: the month
    /// view has nowhere to zoom out to and no neighbour to step to.
    fn is_day(&self) -> bool {
        self.chart().is_some_and(|chart| chart.range == "day")
    }

    /// Whether there is a day after this one to step to. The daemon leaves `next`
    /// empty on today, since nothing is recorded after now.
    fn has_next(&self) -> bool {
        self.is_day() && self.chart().is_some_and(|chart| !chart.next.is_empty())
    }

    /// How wide the canvas has to be, which the daemon decided when it chose a
    /// bar width. Without an explicit width the drawing area is allocated 0px and
    /// nothing appears.
    fn width(&self) -> i32 {
        self.chart().map_or(0, |chart| chart.chart_width as i32)
    }

    /// The hovered bar's tooltip, so the pointer position picks the text rather
    /// than a motion handler pushing it into the widget.
    fn tooltip(&self) -> Option<&str> {
        let chart = self.chart()?;
        let bar = chart.bars.get(self.hover?)?;
        Some(bar.tooltip.as_str())
    }

    /// Ask the daemon for `view` and show its answer.
    ///
    /// One round trip on the GTK thread, as the old popup did. It is a unix
    /// socket on the same machine answering from memory, and it only happens on a
    /// click — a command would buy an intermediate "loading" state nobody would
    /// see.
    fn show(&mut self, view: View) {
        self.chart = match client::request(&self.paths, &Request::Chart { view: view.clone() }) {
            Ok(Response::Chart(chart)) => Some(Rc::new(chart)),
            Ok(other) => {
                eprintln!("claude-usage-gui: unexpected response: {other:?}");
                None
            }
            Err(e) => {
                eprintln!("claude-usage-gui: daemon unreachable: {e:#}");
                None
            }
        };
        self.view = view;
        // The new chart has its own bars, so an index into the old one points at
        // an unrelated day. The next motion event will set it again.
        self.hover = None;
    }

    /// The day a click at `x` asks for, if that gesture means anything here.
    ///
    /// `None` in the day view, past the last column, and on a bar with no date —
    /// which is how the daemon marks the bars of a day view, so this stays right
    /// even if the range check above it ever changes.
    fn day_at(&self, x: f64) -> Option<NaiveDate> {
        let chart = self.chart().filter(|chart| chart.range == "month")?;
        let bar = chart.bars.get(plot::bar_at(chart, x)?)?;
        NaiveDate::parse_from_str(&bar.date, "%Y-%m-%d").ok()
    }
}

#[relm4::component(pub)]
impl Component for Popup {
    type Init = ();
    type Input = PopupMsg;
    type Output = ();
    type CommandOutput = ();

    view! {
        gtk::Window {
            set_title: Some("Claude usage"),

            // A layer surface has no titlebar to close from, and Escape is the
            // reflex for a popup. Needs the keyboard mode `ui::place` sets, or no
            // key event reaches this at all.
            add_controller = gtk::EventControllerKey {
                connect_key_pressed[sender] => move |_, key, _, _| {
                    if key == gtk::gdk::Key::Escape {
                        sender.input(PopupMsg::Close);
                    }
                    gtk::glib::Propagation::Proceed
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                add_css_class: "widgets-popup",

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    add_css_class: "widgets-head",
                    set_margin_bottom: 10,

                    gtk::Button {
                        set_label: "󰅃",
                        // 6px here *and* in the stylesheet, as the popup this
                        // replaced had it — the two add up, and dropping one
                        // halves the gaps between the nav buttons.
                        set_margin_end: 6,
                        set_tooltip_text: Some("the whole month"),
                        add_css_class: "widgets-button",
                        #[watch]
                        set_sensitive: model.is_day(),
                        connect_clicked => PopupMsg::Month,
                    },
                    gtk::Button {
                        set_label: "‹",
                        // 6px here *and* in the stylesheet, as the popup this
                        // replaced had it — the two add up, and dropping one
                        // halves the gaps between the nav buttons.
                        set_margin_end: 6,
                        set_tooltip_text: Some("the day before"),
                        add_css_class: "widgets-button",
                        #[watch]
                        set_sensitive: model.is_day(),
                        connect_clicked => PopupMsg::Step(-1),
                    },
                    gtk::Button {
                        set_label: "›",
                        // 6px here *and* in the stylesheet, as the popup this
                        // replaced had it — the two add up, and dropping one
                        // halves the gaps between the nav buttons.
                        set_margin_end: 6,
                        set_tooltip_text: Some("the day after"),
                        add_css_class: "widgets-button",
                        #[watch]
                        set_sensitive: model.has_next(),
                        connect_clicked => PopupMsg::Step(1),
                    },

                    gtk::Label {
                        add_css_class: "widgets-title",
                        set_margin_start: 4,
                        #[watch]
                        set_label: model.title(),
                    },

                    // Pushes the close button to the far edge.
                    gtk::Box { set_hexpand: true },

                    gtk::Button {
                        set_label: "✕",
                        add_css_class: "widgets-button",
                        connect_clicked => PopupMsg::Close,
                    },
                },

                #[name(area)]
                gtk::DrawingArea {
                    add_css_class: "usage-chart",
                    set_halign: gtk::Align::Center,
                    set_content_height: CHART_HEIGHT + LABEL_HEIGHT,
                    #[watch]
                    set_content_width: model.width(),
                    // Follows the hovered bar, so the pointer sets one piece of
                    // state and both the tooltip and the highlight come off it.
                    #[watch]
                    set_tooltip_text: model.tooltip(),

                    add_controller = gtk::EventControllerMotion {
                        connect_motion[sender] => move |_, x, _| {
                            sender.input(PopupMsg::Hover(Some(x)));
                        },
                        connect_leave[sender] => move |_| {
                            sender.input(PopupMsg::Hover(None));
                        },
                    },
                    add_controller = gtk::GestureClick {
                        connect_pressed[sender] => move |_, _, x, _| {
                            sender.input(PopupMsg::Drill(x));
                        },
                    },
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        ui::place(&root);
        ui::load_css();

        let today = View::today();
        let mut model = Popup {
            paths: Paths::from_env(),
            view: today.clone(),
            chart: None,
            hover: None,
        };
        model.show(today);

        let widgets = view_output!();
        // The first frame. `post_view` covers every one after it, but the macro
        // does not run it as part of building the widgets.
        paint(&widgets.area, &model);
        root.set_visible(true);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            PopupMsg::Close => root.close(),
            PopupMsg::Month => self.show(View::Month),
            PopupMsg::Step(days) => {
                // Only from a day, and only to a date that exists — the checked
                // add is what keeps a held-down ‹ from panicking at the end of
                // the calendar rather than simply stopping.
                if let View::Day(day) = self.view {
                    if let Some(day) = day.checked_add_signed(Duration::days(days)) {
                        self.show(View::Day(day));
                    }
                }
            }
            PopupMsg::Drill(x) => {
                if let Some(day) = self.day_at(x) {
                    self.show(View::Day(day));
                }
            }
            PopupMsg::Hover(x) => {
                self.hover = x
                    .zip(self.chart())
                    .and_then(|(x, chart)| plot::bar_at(chart, x));
            }
        }
    }

    /// Re-render the canvas after the widgets have taken their new values.
    fn post_view(&self) {
        paint(&widgets.area, self);
    }
}

/// Hand the canvas a closure that draws the current state, and ask for a frame.
///
/// This is the commit step for the one part of the window that is not widgets. A
/// `DrawingArea` has no property holding its contents for Relm4 to diff, so
/// "render" here means installing a fresh closure over the state and calling
/// `queue_draw` — which is why [`plot::draw`] is pure and why the chart lives
/// behind an `Rc`: the closure is rebuilt on every pass, so capturing it has to be
/// a refcount bump.
///
/// `set_draw_func` on its own is not enough. GTK's contract is that it sets the
/// callback and you say when the drawing changed, so without the `queue_draw` a
/// new closure would sit unused until something else happened to invalidate the
/// widget.
fn paint(area: &gtk::DrawingArea, model: &Popup) {
    let chart = model.chart.clone();
    let hover = model.hover;
    area.set_draw_func(move |_, ctx, _, _| {
        if let Some(chart) = &chart {
            plot::draw(chart, hover, ctx);
        }
    });
    area.queue_draw();
}
