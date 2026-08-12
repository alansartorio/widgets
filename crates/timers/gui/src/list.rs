//! The timer list: what the bar's left click opens.
//!
//! The window follows the daemon live. A command holds a [`Subscription`] open
//! on a background thread and forwards every snapshot it publishes, so a timer
//! added from the shell or from the add sheet shows up as fast as the daemon
//! knows about it.
//!
//! There is no local clock. The daemon publishes on every whole second for as
//! long as any timer exists — which is exactly when a countdown is on screen — so
//! its snapshots *are* the tick, and they carry its own rendered `label` rather
//! than a second opinion computed here. `sync` keys the rows on timer id so that
//! once-a-second update costs a label assignment per row instead of a rebuild.

use std::time::Duration;

use relm4::factory::{DynamicIndex, FactoryVecDeque};
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use timers_core::client;
use timers_core::models::Status;
use timers_core::paths::{Paths, TimersPaths};
use timers_core::proto::{Request, Response};

use crate::row::{Row, RowOutput};
use crate::ui;

/// How tall the list of rows is allowed to get before it starts scrolling.
///
/// A row is a little over 50px, so this is eight rows whole and a sliver of the
/// ninth — enough that the sliver itself says "there is more below" without a
/// scrollbar having to. Above that the popup would start crowding the bottom of
/// a laptop screen, which is worse than scrolling.
const MAX_LIST_HEIGHT: i32 = 440;

/// How long the subscriber waits before dialling a daemon that was not there.
/// Long enough not to spin, short enough that the list repopulates on its own
/// while you are still looking at it.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

pub struct List {
    paths: Paths,
    status: Status,
    rows: FactoryVecDeque<Row>,
}

#[derive(Debug)]
pub enum ListMsg {
    OpenAddSheet,
    Close,
    Acknowledge(DynamicIndex),
    Cancel(DynamicIndex),
}

#[derive(Debug)]
pub enum ListCmd {
    /// The daemon published; `Status::failed()` when it could not be reached.
    Snapshot(Status),
}

impl List {
    /// Bring the rows in line with the current snapshot, keyed on timer id.
    ///
    /// The daemon publishes every second while anything is counting down, so this
    /// runs about once a second and must not churn widgets: clearing and
    /// re-pushing would destroy the row under the pointer, drop the focus ring,
    /// and restart any animation, every second, forever. Instead each id keeps
    /// its widgets for as long as the timer exists — only genuinely new timers
    /// build any, and a survivor is updated by writing its new view into the
    /// model, which marks that one row for re-render.
    ///
    /// Ids come from an AUTOINCREMENT column and are never reused, so they are
    /// safe to key on — a recycled id would move one timer's widgets onto
    /// another.
    fn sync(&mut self) {
        // Cloned up front so the guard can borrow `self.rows` while this reads
        // the target order.
        let desired = self.status.timers.clone();
        let mut rows = self.rows.guard();

        // Drop rows whose timer is gone.
        let mut position = 0;
        while position < rows.len() {
            let id = rows.get(position).map(|row| row.timer.id);
            if id.is_some_and(|id| desired.iter().any(|t| t.id == id)) {
                position += 1;
            } else {
                rows.remove(position);
            }
        }

        // Then walk the target order, putting the right row at each position.
        // Everything before `target` is already correct, so the search starts
        // there — which also means a row only ever moves forwards.
        for (target, timer) in desired.iter().enumerate() {
            let found = (target..rows.len())
                .find(|&i| rows.get(i).is_some_and(|row| row.timer.id == timer.id));

            match found {
                Some(current) => {
                    // A timer that fires jumps to the top of the daemon's order,
                    // so rows do reorder in practice.
                    if current != target {
                        rows.move_to(current, target);
                    }
                    if let Some(row) = rows.get_mut(target) {
                        row.timer = timer.clone();
                    }
                }
                None => {
                    rows.insert(target, timer.clone());
                }
            }
        }
    }

    /// The timer id at a factory position, if the row is still there.
    ///
    /// A click and the snapshot that removes the row it was on can cross, so
    /// this is deliberately fallible rather than an index into `status.timers`.
    fn id_at(&self, index: &DynamicIndex) -> Option<i64> {
        self.rows.get(index.current_index()).map(|row| row.timer.id)
    }

    /// Send a request and adopt the snapshot it answers with.
    ///
    /// The daemon answers every mutation with the resulting state, so this is
    /// one round trip and not a write followed by a read.
    fn mutate(&mut self, request: Request) {
        match client::request(&self.paths, &request) {
            Ok(Response::Status(status)) => {
                self.status = status;
                self.sync();
            }
            Ok(Response::Error(msg)) => eprintln!("timers-gui: {msg}"),
            Err(e) => eprintln!("timers-gui: daemon unreachable: {e:#}"),
        }
    }
}

#[relm4::component(pub)]
impl Component for List {
    type Init = ();
    type Input = ListMsg;
    type Output = ();
    type CommandOutput = ListCmd;

    view! {
        gtk::Window {
            set_title: Some("Timers"),

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                add_css_class: "widgets-popup",

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    add_css_class: "widgets-head",

                    gtk::Label {
                        set_label: "timers",
                        add_css_class: "widgets-title",
                    },
                    gtk::Box { set_hexpand: true },
                    gtk::Button {
                        set_label: "+",
                        set_tooltip_text: Some("new timer"),
                        add_css_class: "widgets-button",
                        connect_clicked => ListMsg::OpenAddSheet,
                    },
                    gtk::Button {
                        set_label: "✕",
                        add_css_class: "widgets-button",
                        set_margin_start: 6,
                        connect_clicked => ListMsg::Close,
                    },
                },

                // A scroller so thirty timers cannot grow the window off the top
                // of the screen. Its height is pinned per update by `fit`, not by
                // natural-height propagation — see there for why.
                gtk::ScrolledWindow {
                    set_max_content_height: MAX_LIST_HEIGHT,
                    set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),

                    #[local_ref]
                    row_box -> gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                    },
                },

                gtk::Label {
                    set_label: "no timers — press + to add one",
                    add_css_class: "widgets-dim",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.status.timers.is_empty() && !model.status.failed,
                },

                // An unreachable daemon also produces an empty list, and "no
                // timers" would be a lie about it — worse than useless when what
                // you want to know is why the timer you set is not here.
                gtk::Label {
                    set_label: "the timers service is not running",
                    add_css_class: "widgets-error",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.status.failed,
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // OnDemand rather than Exclusive: the list is read and clicked, not typed
        // into, so it should not hold the keyboard away from whatever you were
        // doing just because it is on screen.
        ui::place(&root, ui::Keyboard::OnDemand);
        ui::load_css();

        let paths = Paths::from_env();
        let rows = FactoryVecDeque::builder()
            .launch(gtk::Box::default())
            .forward(sender.input_sender(), |out| match out {
                RowOutput::Acknowledge(id) => ListMsg::Acknowledge(id),
                RowOutput::Cancel(id) => ListMsg::Cancel(id),
            });

        let mut model = List {
            status: fetch(&paths),
            paths: paths.clone(),
            rows,
        };
        model.sync();

        // Follow the daemon. `Subscription::recv` blocks, so this has to be a
        // command on a background thread — on the GTK thread a wedged daemon
        // would freeze the window.
        sender.spawn_command(move |out| {
            loop {
                match client::subscribe(&paths) {
                    Ok(mut sub) => loop {
                        match sub.recv() {
                            Ok(Response::Status(status)) => {
                                if out.send(ListCmd::Snapshot(status)).is_err() {
                                    return;
                                }
                            }
                            Ok(Response::Error(msg)) => eprintln!("timers-gui: {msg}"),
                            // A dead subscription cannot be resumed: drop it and
                            // dial again, the way `timers tail` does.
                            Err(e) => {
                                eprintln!("timers-gui: subscription ended: {e:#}");
                                break;
                            }
                        }
                    },
                    Err(e) => {
                        eprintln!("timers-gui: daemon unreachable: {e:#}");
                        if out.send(ListCmd::Snapshot(Status::failed())).is_err() {
                            return;
                        }
                    }
                }
                std::thread::sleep(RECONNECT_DELAY);
            }
        });

        let row_box = model.rows.widget();
        let widgets = view_output!();
        fit(model.rows.widget());
        root.set_visible(true);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            ListMsg::Close => root.close(),
            ListMsg::OpenAddSheet => spawn_add_window(),
            ListMsg::Acknowledge(index) => {
                if let Some(id) = self.id_at(&index) {
                    self.mutate(Request::Ack { id });
                }
            }
            ListMsg::Cancel(index) => {
                if let Some(id) = self.id_at(&index) {
                    self.mutate(Request::Remove { id });
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _: &Self::Root,
    ) {
        match msg {
            ListCmd::Snapshot(status) => {
                self.status = status;
                self.sync();
            }
        }
    }

    /// Resize the window to whatever the rows now need.
    ///
    /// `post_view` rather than `update_view`, which the macro writes from the
    /// `#[watch]` annotations — its body is inlined *after* those, which is
    /// exactly when the rows are the size they are going to be.
    fn post_view(&self) {
        fit(self.rows.widget());
    }
}

/// Grow (or shrink) the window to fit its rows, up to the cap.
///
/// A layer surface is measured once and committed to the compositor, and nothing
/// afterwards makes GTK revisit it. Combined with a `ScrolledWindow`'s minimum
/// height being 0 — it will happily shrink below its own content — a window that
/// opened with one timer stayed that size and simply clipped the rest, with no
/// scrollbar to say so. Pinning the scroller's minimum *and* the window's
/// default height forces a fresh configure in both directions.
///
/// Everything is reached from the factory's box rather than passed in, so the one
/// caller that has no `Widgets` to hand still works.
fn fit(rows: &gtk::Box) {
    let (_, natural_rows, _, _) = rows.measure(gtk::Orientation::Vertical, -1);

    // The viewport GTK inserts between a ScrolledWindow and its child means the
    // scroller is a grandparent, not a parent.
    if let Some(scroller) = rows
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    {
        scroller.set_min_content_height(natural_rows.min(MAX_LIST_HEIGHT));
    }
    if let Some(win) = rows.root().and_downcast::<gtk::Window>()
        && let Some(child) = win.child()
    {
        let (_, natural, _, _) = child.measure(gtk::Orientation::Vertical, -1);
        win.set_default_height(natural);
    }
}

/// Ask the daemon for the current snapshot, for the very first paint.
///
/// After this the subscription is what keeps the model current; this exists so
/// the window opens already populated rather than empty for a moment.
fn fetch(paths: &Paths) -> Status {
    match client::request(paths, &Request::List) {
        Ok(Response::Status(status)) => status,
        Ok(Response::Error(msg)) => {
            eprintln!("timers-gui: {msg}");
            Status::failed()
        }
        Err(e) => {
            eprintln!("timers-gui: daemon unreachable: {e:#}");
            Status::failed()
        }
    }
}

/// Launch the add sheet as its own process.
///
/// It has a different application id, so it cannot be a window of this one —
/// which is the price of letting the bar's two clicks toggle independently.
fn spawn_add_window() {
    let exe = std::env::current_exe().unwrap_or_else(|_| "timers-gui".into());
    if let Err(e) = std::process::Command::new(exe).arg("add").spawn() {
        eprintln!("timers-gui: could not open the add window: {e:#}");
    }
}
