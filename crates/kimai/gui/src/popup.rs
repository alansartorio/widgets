//! The window: what is running, and how hoy / semana / mes stand.

use std::time::Duration;

use chrono::Local;
use kimai_core::{Config, Output, Poller, describe};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, WidgetTemplate, gtk};

use crate::ui;

/// How often the window asks the poller for a fresh reading.
///
/// One second, like the bar's tick, and for the same reason: the running
/// entry's elapsed time is on screen. Most of those calls do no network at all
/// — the poller decides that from its own config — so this is a repaint rate,
/// not a request rate.
const TICK: Duration = Duration::from_secs(1);

pub struct Popup {
    poller: Option<Poller>,
    /// The last good reading. Seeded from whatever `kimai tail` left behind, so
    /// the first frame has numbers in it.
    output: Output,
    /// Set when the poller itself could not be built — no config, no token —
    /// which is a different failure from a poll that did not answer.
    fatal: Option<String>,
}

#[derive(Debug)]
pub enum PopupMsg {
    Tick,
    Close,
}

impl Popup {
    /// The last line the bar's `kimai tail` wrote.
    ///
    /// A cache, not a protocol: anything wrong with it — absent, truncated,
    /// written by an older version — just means the window opens on the
    /// "cargando…" state its own first tick replaces a moment later.
    fn seed() -> Output {
        std::fs::read_to_string(kimai_core::last_line_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Output>(&raw).ok())
            .unwrap_or_else(|| Output::failed("cargando…"))
    }

    fn delta_class(&self, seconds: i64) -> &'static str {
        if seconds >= 0 { "ahead" } else { "behind" }
    }

    /// The line under the three rows.
    fn footer(&self) -> String {
        if let Some(fatal) = &self.fatal {
            return fatal.clone();
        }
        if self.output.failed {
            return format!("kimai: {}", self.output.error);
        }
        let mut parts = Vec::new();
        if self.output.holiday {
            parts.push(match self.output.holiday_name.as_str() {
                "" => "feriado".to_string(),
                name => format!("feriado — {name}"),
            });
        }
        if self.output.holidays_degraded {
            parts.push("sin datos de feriados".into());
        }
        if parts.is_empty() {
            parts.push(format!(
                "semana {} → {}",
                self.output.week_start, self.output.week_end
            ));
        }
        parts.join(" · ")
    }

    fn footer_class(&self) -> &'static str {
        if self.fatal.is_some() || self.output.failed {
            "warn"
        } else if self.output.holiday {
            "holiday"
        } else {
            ""
        }
    }

    /// A progress fraction for one row. Never divides by zero; see
    /// `kimai_core::output::percent` for why that matters nineteen days a year.
    fn fraction(percent: i64) -> f64 {
        (percent as f64 / 100.0).clamp(0.0, 1.0)
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
            set_title: Some("kimai"),

            // A layer surface has no titlebar to close from, and Escape is the
            // reflex for a popup. Needs the keyboard mode `ui::place` sets, or
            // no key event reaches this at all — and note that with OnDemand
            // they only start arriving once the window has been clicked.
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

                    gtk::Label {
                        add_css_class: "widgets-title",
                        set_label: "kimai",
                    },
                    gtk::Box { set_hexpand: true },
                    gtk::Button {
                        set_label: "✕",
                        add_css_class: "widgets-button",
                        connect_clicked => PopupMsg::Close,
                    },
                },

                // What is running. Hidden rather than emptied when nothing is,
                // so an idle window is three rows rather than three rows and a
                // blank card.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    add_css_class: "kimai-running",
                    #[watch]
                    set_visible: model.output.running,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        set_valign: gtk::Align::Center,

                        gtk::Label {
                            add_css_class: "kimai-task",
                            set_halign: gtk::Align::Start,
                            #[watch]
                            set_label: if model.output.task.is_empty() {
                                "sin proyecto"
                            } else {
                                &model.output.task
                            },
                        },
                        gtk::Label {
                            add_css_class: "kimai-sub",
                            set_halign: gtk::Align::Start,
                            #[watch]
                            set_label: &if model.output.description.is_empty() {
                                format!("desde {}", model.output.since)
                            } else {
                                format!("{} · desde {}", model.output.description, model.output.since)
                            },
                        },
                    },

                    gtk::Label {
                        add_css_class: "kimai-elapsed",
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_label: &model.output.running_time,
                    },
                },

                gtk::Label {
                    add_css_class: "widgets-dim",
                    set_halign: gtk::Align::Start,
                    set_margin_bottom: 8,
                    set_label: "nada corriendo",
                    #[watch]
                    set_visible: !model.output.running && !model.output.failed,
                },

                #[template]
                Row {
                    #[template_child] span { set_label: "hoy" },
                    #[template_child] total {
                        #[watch] set_label: &model.output.today_time,
                    },
                    #[template_child] expected {
                        #[watch] set_label: &format!("/ {}", model.output.expected_today),
                    },
                    #[template_child] delta {
                        #[watch] set_label: &model.output.today_delta,
                        #[watch] set_css_classes: &["kimai-delta", model.delta_class(model.output.today_delta_seconds)],
                    },
                    #[template_child] bar {
                        #[watch] set_fraction: Popup::fraction(model.output.day_percent),
                        #[watch] set_css_classes: if model.output.holiday {
                            &["kimai-bar", "holiday"]
                        } else {
                            &["kimai-bar"]
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] span { set_label: "semana" },
                    #[template_child] total {
                        #[watch] set_label: &model.output.week_time,
                    },
                    #[template_child] expected {
                        #[watch] set_label: &format!("/ {}", model.output.expected_week),
                    },
                    #[template_child] delta {
                        #[watch] set_label: &model.output.week_delta,
                        #[watch] set_css_classes: &["kimai-delta", model.delta_class(model.output.week_delta_seconds)],
                    },
                    #[template_child] bar {
                        #[watch] set_fraction: Popup::fraction(model.output.week_percent),
                    },
                },

                #[template]
                Row {
                    #[template_child] span { set_label: "mes" },
                    #[template_child] total {
                        #[watch] set_label: &model.output.month_time,
                    },
                    #[template_child] expected {
                        #[watch] set_label: &format!("/ {}", model.output.expected_month),
                    },
                    #[template_child] delta {
                        #[watch] set_label: &model.output.month_delta,
                        #[watch] set_css_classes: &["kimai-delta", model.delta_class(model.output.month_delta_seconds)],
                    },
                    #[template_child] bar {
                        #[watch] set_fraction: Popup::fraction(model.output.month_percent),
                    },
                },

                gtk::Label {
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_label: &model.footer(),
                    #[watch]
                    set_css_classes: &["kimai-foot", model.footer_class()],
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

        // A window that cannot poll still opens and says why, rather than
        // flashing and vanishing — the click that raised it should always
        // produce something to read.
        let (poller, fatal) = match Config::load() {
            Ok(Some(cfg)) => match Poller::new(cfg) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(describe(&e))),
            },
            Ok(None) => (
                None,
                Some("kimai no está configurado en esta máquina".into()),
            ),
            Err(e) => (None, Some(describe(&e))),
        };

        let model = Popup {
            poller,
            output: Self::seed(),
            fatal,
        };

        // The clock has to keep moving while the window is up, and the poller's
        // own cadence decides which ticks cost a request. A GLib timeout rather
        // than a thread because everything it touches is on this one.
        let tick = sender.input_sender().clone();
        gtk::glib::timeout_add_local(TICK, move || {
            tick.send(PopupMsg::Tick).ok();
            gtk::glib::ControlFlow::Continue
        });

        let widgets = view_output!();
        root.set_visible(true);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            PopupMsg::Close => root.close(),
            PopupMsg::Tick => {
                let Some(poller) = self.poller.as_mut() else {
                    return;
                };
                match poller.tick(Local::now()) {
                    Ok(output) => self.output = output,
                    // Keep the numbers on screen and say what went wrong under
                    // them. Blanking a window because one poll timed out throws
                    // away the reading that is still broadly true.
                    Err(e) => self.output = Output::failed(&describe(&e)),
                }
            }
        }
    }
}

/// One span's row: label, total, expected, delta, and a bar under all of it.
///
/// A template because the three are identical but for their bindings, and
/// writing the same eight widgets out three times is how the middle one ends up
/// subtly different.
#[relm4::widget_template(pub)]
impl WidgetTemplate for Row {
    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            add_css_class: "kimai-row",

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,

                #[name = "span"]
                gtk::Label {
                    add_css_class: "kimai-span",
                    set_halign: gtk::Align::Start,
                    // Fixed width, so the three totals line up as a column
                    // rather than wandering with the length of "semana".
                    set_width_chars: 7,
                    set_xalign: 0.0,
                },
                #[name = "total"]
                gtk::Label {
                    add_css_class: "kimai-total",
                    set_halign: gtk::Align::Start,
                },
                #[name = "expected"]
                gtk::Label {
                    add_css_class: "kimai-expected",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_xalign: 0.0,
                },
                #[name = "delta"]
                gtk::Label {
                    add_css_class: "kimai-delta",
                    set_halign: gtk::Align::End,
                },
            },

            #[name = "bar"]
            gtk::ProgressBar {
                add_css_class: "kimai-bar",
                set_margin_top: 4,
            },
        }
    }
}
