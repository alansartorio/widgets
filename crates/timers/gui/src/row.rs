//! One row of the timer list, as a factory component.
//!
//! A pure renderer of one [`TimerView`]. It holds no clock and no derived state:
//! the countdown text, the percentage and the "fired N ago" line were all
//! computed by the daemon, and the list hands each row its new view whenever a
//! snapshot lands. `#[watch]` then re-renders exactly the labels whose source
//! moved.

use relm4::factory::{DynamicIndex, FactoryComponent, FactorySender};
use relm4::gtk;
use relm4::gtk::prelude::*;
use timers_core::models::{TimerView, fmt_remaining};

#[derive(Debug)]
pub struct Row {
    pub timer: TimerView,
}

/// What a row asks the list to do.
///
/// A row never talks to the daemon itself: one place owns that conversation, and
/// it is the component that also owns the snapshot the answer replaces. The
/// position rather than the timer's id, because that is what a factory child can
/// hand back — the list resolves it against the row it belongs to.
#[derive(Debug)]
pub enum RowOutput {
    Acknowledge(DynamicIndex),
    Cancel(DynamicIndex),
}

impl Row {
    /// The line under the name: when it lands, or how long ago it went off, plus
    /// whether its name is kept off the bar.
    ///
    /// The hidden marker earns its place because the bar calls every hidden timer
    /// `timer` — without it here, there is nowhere at all to see which of your
    /// timers that was. Plain text rather than a glyph, since this window
    /// inherits whatever font the theme resolves and cannot assume the Nerd Font
    /// the bar sets for itself.
    fn subtitle(&self) -> String {
        let when = if self.timer.firing {
            format!(
                "fired {} ago",
                fmt_remaining(self.timer.remaining).trim_start_matches('+')
            )
        } else {
            format!("at {}", self.timer.at)
        };
        if self.timer.private {
            format!("{when} · hidden")
        } else {
            when
        }
    }

    fn row_class(&self) -> &'static str {
        if self.timer.firing { "firing" } else { "" }
    }
}

#[relm4::factory(pub)]
impl FactoryComponent for Row {
    type Init = TimerView;
    /// Nothing to send a row: the list updates it by writing its new view
    /// straight into the model, which marks it for re-render.
    type Input = ();
    type Output = RowOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;

    view! {
        root = gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            #[watch]
            set_css_classes: &["timer-row", self.row_class()],

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,

                gtk::Label {
                    add_css_class: "timer-name",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_label: &self.timer.name,
                },
                gtk::Label {
                    add_css_class: "timer-sub",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_label: &self.subtitle(),
                },
            },

            // The daemon already rendered this — see `TimerView::label`.
            gtk::Label {
                add_css_class: "timer-remaining",
                #[watch]
                set_label: &self.timer.label,
            },

            // Acknowledge: only meaningful once it has gone off, so it appears
            // then. A plain ✓ rather than a Nerd Font glyph — the bar can assume
            // that font because it sets it explicitly, but this window inherits
            // whatever the theme resolves and a missing glyph is a blank box.
            gtk::Button {
                set_label: "✓",
                set_tooltip_text: Some("acknowledge"),
                add_css_class: "widgets-button",
                add_css_class: "primary",
                set_valign: gtk::Align::Center,
                #[watch]
                set_visible: self.timer.firing,
                connect_clicked[sender, index] => move |_| {
                    let _ = sender.output(RowOutput::Acknowledge(index.clone()));
                },
            },

            // Exactly one of the two shows at a time. Acknowledging a fired timer
            // deletes it, so on a row that has already gone off the ✕ would be a
            // second button doing the same thing — and the one you would rather
            // not reach for by accident, since it reads as "cancel" on the rows
            // where it does mean that.
            gtk::Button {
                set_label: "✕",
                set_tooltip_text: Some("cancel"),
                add_css_class: "widgets-button",
                add_css_class: "danger",
                set_margin_start: 6,
                set_valign: gtk::Align::Center,
                #[watch]
                set_visible: !self.timer.firing,
                connect_clicked[sender, index] => move |_| {
                    let _ = sender.output(RowOutput::Cancel(index.clone()));
                },
            },
        }
    }

    fn init_model(timer: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { timer }
    }
}
