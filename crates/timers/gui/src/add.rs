//! The add sheet: what the bar's right click opens.
//!
//! Name, a finish expressed either way — a duration from now, or a time of day —
//! and whether to keep the name off the bar. The two time fields sit behind a
//! toggle rather than being one clever field, so there is never a question about
//! how what you typed was read.

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use timers_core::client;
use timers_core::paths::{Paths, TimersPaths};
use timers_core::proto::{Request, Response};
use timers_core::timespec;

use crate::ui;

/// Which way the finish is being given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A duration from now.
    In,
    /// A time of day.
    At,
}

impl Mode {
    fn flipped(self) -> Self {
        match self {
            Self::In => Self::At,
            Self::At => Self::In,
        }
    }

    /// The examples under the field, which have to follow the mode or they
    /// describe the wrong syntax.
    fn hint(self) -> &'static str {
        match self {
            Self::In => "25m · 1h30m · 90s · 5:00",
            Self::At => "1530 · 15:30 · 153045 · next time the clock reads it",
        }
    }

    fn stack_child(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::At => "at",
        }
    }

    /// What to say when the field is empty — worth naming the mode, since the
    /// answer differs.
    fn missing(self) -> &'static str {
        match self {
            Self::In => "how long? (try 25m)",
            Self::At => "when should it go off? (try 1530)",
        }
    }
}

/// Where the cursor should go after an update, once.
///
/// A `Cell` so `post_view` can take it: `post_view` only gets `&self`, and the
/// request has to be consumed rather than reapplied — it runs after *every*
/// update, and re-grabbing focus on each one would fight the Tab key.
#[derive(Debug, Clone, Copy)]
enum Focus {
    /// Stay on the mode control, so the next Tab moves on from it. What a
    /// keyboard flip wants.
    Mode,
    /// Into the field the mode just revealed. What a click wants — it already
    /// said which mode, so the next thing to do is type.
    Spec,
}

pub struct Add {
    paths: Paths,
    mode: Mode,
    /// Empty until something goes wrong, which is also how the label's
    /// visibility is decided.
    error: String,
    focus: std::cell::Cell<Option<Focus>>,
}

#[derive(Debug)]
pub enum AddMsg {
    /// Pick a mode outright — what clicking a segment means.
    SetMode(Mode),
    /// Switch to the other one — what space on the focused control means.
    FlipMode,
    /// Name, then whichever spec field is showing, then the hide checkbox.
    Submit(String, String, bool),
    Close,
}

#[relm4::component(pub)]
impl SimpleComponent for Add {
    type Init = ();
    type Input = AddMsg;
    type Output = ();

    view! {
        window = gtk::Window {
            set_title: Some("New timer"),

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                add_css_class: "widgets-popup",

                gtk::Label {
                    set_label: "new timer",
                    add_css_class: "widgets-title",
                    set_halign: gtk::Align::Start,
                    set_margin_bottom: 10,
                },

                // Enter in any of the three fields submits; that is wired in
                // `init`, where the other fields' values are reachable.
                #[name(name)]
                gtk::Entry {
                    add_css_class: "timers-entry",
                    set_placeholder_text: Some("name"),
                },

                // in | at
                #[name(modes)]
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    add_css_class: "timers-modes",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 10,

                    // One Tab stop for the pair, not two: only the *selected*
                    // segment stays in the tab chain, and the key handler in
                    // `init` turns space into a flip. As a radio group each
                    // segment was its own stop and space on the selected one did
                    // nothing — correct radio behaviour, useless for a control
                    // with exactly two answers.
                    //
                    // Making the containing box focusable instead would read
                    // better and does not work: a gtk::Box with `focusable` set
                    // is skipped by Tab anyway, which sends the keystrokes meant
                    // for the toggle into the field below it.
                    #[name(in_mode)]
                    gtk::ToggleButton {
                        set_label: "in",
                        add_css_class: "timers-toggle",
                        // Not on click: a click already says which mode you want,
                        // and the cursor belongs in the field it reveals.
                        set_focus_on_click: false,
                        #[watch]
                        set_active: model.mode == Mode::In,
                        #[watch]
                        set_focusable: model.mode == Mode::In,
                        connect_clicked => AddMsg::SetMode(Mode::In),
                    },
                    #[name(at_mode)]
                    gtk::ToggleButton {
                        set_label: "at",
                        add_css_class: "timers-toggle",
                        set_group: Some(&in_mode),
                        set_focus_on_click: false,
                        set_focusable: false,
                        #[watch]
                        set_active: model.mode == Mode::At,
                        #[watch]
                        set_focusable: model.mode == Mode::At,
                        connect_clicked => AddMsg::SetMode(Mode::At),
                    },
                },

                #[name(stack)]
                gtk::Stack {
                    // No transition, deliberately. A crossfade keeps the
                    // outgoing child mapped and focusable for its duration, so a
                    // Tab arriving in that window lands on the field that is
                    // fading *out* and the text goes into a widget you cannot
                    // see. Switching instantly costs nothing on a two-field swap.
                    set_transition_type: gtk::StackTransitionType::None,
                    set_margin_top: 8,

                    #[name(spec_in)]
                    add_named[Some("in")] = &gtk::Entry {
                        add_css_class: "timers-entry",
                        set_placeholder_text: Some("25m"),
                    },
                    #[name(spec_at)]
                    add_named[Some("at")] = &gtk::Entry {
                        add_css_class: "timers-entry",
                        set_placeholder_text: Some("1530"),
                    },

                    // After the children, not before: properties are applied in
                    // the order written, and naming a child that has not been
                    // added yet is a runtime warning from GTK.
                    #[watch]
                    set_visible_child_name: model.mode.stack_child(),
                },

                gtk::Label {
                    add_css_class: "timers-hint",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_label: model.mode.hint(),
                },

                // After the when-block rather than under the name, even though it
                // is a property of the name: putting it there would break the
                // type-name, Tab, type-time run this window exists to make fast.
                #[name(private)]
                gtk::CheckButton {
                    set_label: Some("hide name on the bar"),
                    add_css_class: "timers-check",
                    set_margin_top: 10,
                    set_tooltip_text: Some(
                        "the bar and the notification say \"timer\" — this list still shows the name",
                    ),
                },

                gtk::Label {
                    add_css_class: "widgets-error",
                    set_halign: gtk::Align::Start,
                    set_wrap: true,
                    #[watch]
                    set_label: &model.error,
                    #[watch]
                    set_visible: !model.error.is_empty(),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_margin_top: 12,

                    gtk::Box { set_hexpand: true },
                    gtk::Button {
                        set_label: "cancel",
                        add_css_class: "widgets-button",
                        set_margin_end: 6,
                        connect_clicked => AddMsg::Close,
                    },
                    #[name(start)]
                    gtk::Button {
                        set_label: "start",
                        add_css_class: "widgets-button",
                        add_css_class: "primary",
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
        // Exclusive: the sheet is here to be typed into, and Escape and Enter
        // both dismiss it, so holding the keyboard for its lifetime cannot
        // strand you.
        ui::place(&root, ui::Keyboard::Exclusive);
        ui::load_css();

        let model = Add {
            paths: Paths::from_env(),
            mode: Mode::In,
            error: String::new(),
            focus: std::cell::Cell::new(None),
        };
        let widgets = view_output!();

        // Space (and the arrows) on the focused mode box. Capture phase, or the
        // box's own key handling would get there first.
        {
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let to = sender.input_sender().clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                use gtk::gdk::Key;
                let msg = match key {
                    Key::space | Key::KP_Space => AddMsg::FlipMode,
                    // Left and Right *select* rather than flip: on a two-segment
                    // control they read as positions, not as a step.
                    Key::Left | Key::Up => AddMsg::SetMode(Mode::In),
                    Key::Right | Key::Down => AddMsg::SetMode(Mode::At),
                    _ => return gtk::glib::Propagation::Proceed,
                };
                let _ = to.send(msg);
                gtk::glib::Propagation::Stop
            });
            widgets.modes.add_controller(keys);
        }

        // Escape closes. A layer-shell window has no titlebar to close from, so
        // without this the only way out is cancel or another click on the bar.
        {
            let keys = gtk::EventControllerKey::new();
            let to = sender.input_sender().clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    let _ = to.send(AddMsg::Close);
                }
                gtk::glib::Propagation::Proceed
            });
            root.add_controller(keys);
        }

        // Enter from any field submits, and so does the start button, so the whole
        // thing is type-name, tab, type-time, enter.
        {
            let submit = {
                let name = widgets.name.clone();
                let spec_in = widgets.spec_in.clone();
                let spec_at = widgets.spec_at.clone();
                let private = widgets.private.clone();
                let to = sender.input_sender().clone();
                move |mode_is_at: &gtk::ToggleButton| {
                    let spec = if mode_is_at.is_active() {
                        spec_at.text()
                    } else {
                        spec_in.text()
                    };
                    let _ = to.send(AddMsg::Submit(
                        name.text().to_string(),
                        spec.to_string(),
                        private.is_active(),
                    ));
                }
            };
            for entry in [&widgets.name, &widgets.spec_in, &widgets.spec_at] {
                let submit = submit.clone();
                let at_mode = widgets.at_mode.clone();
                entry.connect_activate(move |_| submit(&at_mode));
            }
            let at_mode = widgets.at_mode.clone();
            widgets
                .start
                .connect_clicked(move |_| submit(&at_mode.clone()));
        }

        root.set_visible(true);
        widgets.name.grab_focus();
        ComponentParts { model, widgets }
    }

    /// Move the cursor where the update asked it to go.
    ///
    /// `post_view`, so it runs after the `#[watch]` that hands the tab stop to
    /// the newly selected segment — grabbing focus on a widget that is not
    /// focusable yet does nothing.
    fn post_view(&self) {
        match self.focus.take() {
            Some(Focus::Mode) => match self.mode {
                Mode::In => widgets.in_mode.grab_focus(),
                Mode::At => widgets.at_mode.grab_focus(),
            },
            Some(Focus::Spec) => match self.mode {
                Mode::In => widgets.spec_in.grab_focus(),
                Mode::At => widgets.spec_at.grab_focus(),
            },
            None => false,
        };
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            // A click: the mode is settled, so the cursor belongs in the field.
            AddMsg::SetMode(mode) => {
                self.mode = mode;
                self.focus.set(Some(Focus::Spec));
            }
            // Space: stay on the control, so the next space flips again and the
            // next Tab moves on to the field.
            AddMsg::FlipMode => {
                self.mode = self.mode.flipped();
                self.focus.set(Some(Focus::Mode));
            }
            AddMsg::Close => relm4::main_application().quit(),
            AddMsg::Submit(name, spec, private) => match self.create(&name, &spec, private) {
                // Staying open with the text intact on failure: the fix is almost
                // always one character, and closing would throw the name away
                // with it.
                Ok(()) => relm4::main_application().quit(),
                Err(message) => self.error = message,
            },
        }
    }
}

impl Add {
    /// Parse the form and ask the daemon to create the timer.
    ///
    /// Errors come back as a sentence to show in the sheet — this is the one
    /// place in the GUI where a failure is the person's to fix rather than
    /// something to degrade around.
    fn create(&self, name: &str, spec: &str, private: bool) -> Result<(), String> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(self.mode.missing().to_string());
        }

        let deadline = match self.mode {
            Mode::At => timespec::parse_at(spec),
            Mode::In => timespec::parse_in(spec).map(|secs| timers_core::now() + secs),
        }
        .map_err(|e| format!("{e:#}"))?;

        let name = name.trim();
        let name = if name.is_empty() { "timer" } else { name };

        match client::request(
            &self.paths,
            &Request::Add {
                name: name.to_string(),
                deadline,
                private,
            },
        ) {
            Ok(Response::Status(_)) => Ok(()),
            Ok(Response::Error(msg)) => Err(msg),
            Err(e) => Err(format!("the timers daemon is not answering: {e:#}")),
        }
    }
}
