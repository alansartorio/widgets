//! The timer, and the snapshot every client renders.
//!
//! [`Timer`] is a database row. [`Status`] is what the daemon publishes: the
//! whole list plus everything derived from "now", so no client has to know how
//! a countdown is formatted or which timer the bar's one line should describe.
//! [`Status::from_timers`] is the single place any of that is decided.

use serde::{Deserialize, Serialize};

/// What the bar shows in place of a hidden timer's name.
///
/// Deliberately the same word [`Status::from_timers`]'s caller gives a nameless
/// timer: a hidden timer then looks exactly like one you never bothered to name,
/// rather than announcing that there is something here worth hiding. A visibly
/// redacted `••••` would be clearer to you and also clearer to everyone watching
/// the screen you are sharing.
pub const MASKED_NAME: &str = "timer";

/// One row of the `timers` table.
///
/// `deadline` and `created_at` are unix seconds — wall clock, not monotonic, so
/// a timer survives a suspend that spans its own deadline and fires the moment
/// the machine is back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timer {
    pub id: i64,
    pub name: String,
    pub deadline: i64,
    pub created_at: i64,
    /// When the notification went out. `None` means still counting down;
    /// `Some(_)` means fired and not yet acknowledged. Acknowledging deletes
    /// the row, so there is no third state and no `acked` column.
    pub fired_at: Option<i64>,
    /// Keep this name off the bar: the pill, its tooltip and the notification
    /// all say [`MASKED_NAME`] instead. The popup still shows the real name,
    /// because you open that deliberately and the bar is just always there.
    ///
    /// Set when the timer is created and not changeable afterwards.
    pub private: bool,
}

impl Timer {
    /// Fired and not yet acknowledged — the state that colours the bar.
    pub fn firing(&self) -> bool {
        self.fired_at.is_some()
    }

    /// Whole seconds until the deadline; negative once it has passed.
    pub fn remaining(&self, now: i64) -> i64 {
        self.deadline - now
    }

    /// How far along the timer is, 0–100, clamped at both ends.
    ///
    /// Measured from `created_at` rather than from a nominal duration so that a
    /// timer created by target time (`--at 15:30`) fills its ring over however
    /// long that turned out to be, instead of jumping.
    pub fn pct(&self, now: i64) -> i64 {
        let total = self.deadline - self.created_at;
        if total <= 0 {
            // Created at or after its own deadline: nothing to fill in, and
            // dividing by this would panic.
            return 100;
        }
        let done = (now - self.created_at).clamp(0, total);
        // Integer math is deliberate — the ring only renders whole percents.
        (done * 100 / total).clamp(0, 100)
    }
}

/// One timer as the bar and the GUI read it: the stored fields plus everything
/// derived from "now", so no consumer has to do time arithmetic.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimerView {
    pub id: i64,
    pub name: String,
    pub deadline: i64,
    /// The wall-clock time it lands, `"15:04"` — local time, since it is only
    /// ever shown to the person sitting in front of it.
    pub at: String,
    /// Seconds left; negative once overdue.
    pub remaining: i64,
    /// The countdown as text: `"5:12"`, `"1:05:03"`, or `"+0:12"` when overdue.
    pub label: String,
    pub firing: bool,
    pub pct: i64,
    /// `name` above is the real one, which is what the popup shows. This says
    /// the bar-facing strings were masked, so the popup can mark the row as
    /// hidden — otherwise the bar reads `timer` and you have no way to tell
    /// which of your timers that is.
    pub private: bool,
}

impl TimerView {
    pub fn of(timer: &Timer, now: i64) -> Self {
        let remaining = timer.remaining(now);
        Self {
            id: timer.id,
            name: timer.name.clone(),
            deadline: timer.deadline,
            at: fmt_clock(timer.deadline),
            remaining,
            label: fmt_remaining(remaining),
            firing: timer.firing(),
            pct: timer.pct(now),
            private: timer.private,
        }
    }

    /// The name as the *bar* may see it. The one chokepoint for masking, so
    /// there is exactly one place to get it wrong.
    fn bar_name(&self) -> &str {
        if self.private {
            MASKED_NAME
        } else {
            &self.name
        }
    }
}

/// The daemon's published snapshot: what `timers list` prints, what every
/// `timers tail` line carries, and what the GUI renders.
///
/// Every field is always present and never null, so eww's `?.` access never has
/// to reach through a missing value — the widget can read `pct` on a snapshot
/// that came from a daemon which is not running.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// The daemon answered.
    pub ok: bool,
    /// The client could not reach the daemon at all.
    pub failed: bool,
    /// Every timer, fired or not.
    pub count: i64,
    /// Timers still counting down.
    pub pending: i64,
    /// Timers that have fired and not been acknowledged.
    pub firing: i64,
    /// `"firing" | "running" | "idle" | "failed"` — one string for the bar to
    /// switch a CSS class on, so the widget does not reimplement the
    /// precedence below in yuck.
    pub state: String,
    /// The name of the timer `label`, `remaining` and `pct` describe. Empty
    /// when idle.
    pub name: String,
    /// The headline countdown text.
    pub label: String,
    pub remaining: i64,
    pub pct: i64,
    /// One line per timer, for the widget's hover.
    pub tooltip: String,
    /// When this snapshot was computed, unix seconds.
    pub at: i64,
    pub timers: Vec<TimerView>,
}

impl Status {
    /// Build the published snapshot from the daemon's timer list.
    ///
    /// Headline precedence: the timer that has been waiting for acknowledgement
    /// longest, else the one about to go off soonest, else nothing. A fired
    /// timer outranks a pending one however far off the pending one is — an
    /// unacknowledged alarm is the whole reason to look at the widget.
    pub fn from_timers(timers: &[Timer], now: i64) -> Self {
        let mut views: Vec<TimerView> = timers.iter().map(|t| TimerView::of(t, now)).collect();
        // Firing first, oldest alarm at the top; then pending, soonest first.
        // The daemon's list is already in this order, but a caller that built
        // one by hand should get the same answer.
        views.sort_by_key(|v| (!v.firing, v.deadline));

        let firing = views.iter().filter(|v| v.firing).count() as i64;
        let count = views.len() as i64;
        let head = views.first();

        let state = if firing > 0 {
            "firing"
        } else if count > 0 {
            "running"
        } else {
            "idle"
        };

        Self {
            ok: true,
            failed: false,
            count,
            pending: count - firing,
            firing,
            state: state.to_string(),
            // Masked for the bar, while `timers` below keeps the real names for
            // the popup.
            name: head.map(|v| v.bar_name().to_string()).unwrap_or_default(),
            label: head.map(|v| v.label.clone()).unwrap_or_default(),
            remaining: head.map(|v| v.remaining).unwrap_or_default(),
            pct: head.map(|v| v.pct).unwrap_or_default(),
            tooltip: tooltip(&views),
            at: now,
            timers: views,
        }
    }

    /// What a client prints when it cannot reach the daemon.
    ///
    /// The CLI keeps a byte-identical literal of this so it can say so without
    /// serializing anything; see `FALLBACK` in the CLI and the test that pins
    /// the two together.
    pub fn failed() -> Self {
        Self {
            ok: false,
            failed: true,
            state: "failed".to_string(),
            label: "timers daemon unreachable".to_string(),
            tooltip: "no answer from the timers daemon · see eww logs".to_string(),
            ..Self::default()
        }
    }
}

/// `"5:12"`, `"1:05:03"`, or `"+1:12"` once the deadline has passed.
///
/// Minutes are not zero-padded but seconds always are, so the text reads like a
/// clock and does not change width every tick.
pub fn fmt_remaining(remaining: i64) -> String {
    let overdue = remaining < 0;
    let secs = remaining.unsigned_abs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    let sign = if overdue { "+" } else { "" };
    if h > 0 {
        format!("{sign}{h}:{m:02}:{s:02}")
    } else {
        format!("{sign}{m}:{s:02}")
    }
}

/// A unix timestamp as local `"15:04"`.
fn fmt_clock(at: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(at, 0) {
        chrono::offset::LocalResult::Single(t) => t.format("%H:%M").to_string(),
        // Ambiguous (a DST fold) or out of range. Neither is worth a Result on
        // a display string; the timer is still perfectly usable without it.
        _ => String::new(),
    }
}

/// The widget's hover text: one line per timer, in the same order as the list.
///
/// Masked like the headline, and not as an afterthought: the tooltip needs no
/// click, so an unmasked one would spill every name the moment the pointer
/// crossed the pill.
fn tooltip(views: &[TimerView]) -> String {
    if views.is_empty() {
        return "no timers · right click to add one".to_string();
    }
    views
        .iter()
        .map(|v| {
            if v.firing {
                format!(
                    "{} · fired {} ago",
                    v.bar_name(),
                    fmt_remaining(v.remaining).trim_start_matches('+')
                )
            } else {
                format!("{} · {} ({})", v.bar_name(), v.label, v.at)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer(id: i64, name: &str, created_at: i64, deadline: i64, fired_at: Option<i64>) -> Timer {
        Timer {
            id,
            name: name.to_string(),
            deadline,
            created_at,
            fired_at,
            private: false,
        }
    }

    fn private(
        id: i64,
        name: &str,
        created_at: i64,
        deadline: i64,
        fired_at: Option<i64>,
    ) -> Timer {
        Timer {
            private: true,
            ..timer(id, name, created_at, deadline, fired_at)
        }
    }

    #[test]
    fn remaining_formats_like_a_clock() {
        assert_eq!(fmt_remaining(312), "5:12");
        assert_eq!(fmt_remaining(3903), "1:05:03");
        assert_eq!(fmt_remaining(0), "0:00");
        assert_eq!(fmt_remaining(-72), "+1:12");
    }

    #[test]
    fn pct_is_clamped_and_never_divides_by_zero() {
        let t = timer(1, "tea", 100, 200, None);
        assert_eq!(t.pct(100), 0);
        assert_eq!(t.pct(150), 50);
        assert_eq!(t.pct(500), 100, "overdue stays at 100, never past it");
        // Deadline at creation time: the interesting degenerate case, since the
        // obvious implementation panics here.
        assert_eq!(timer(2, "now", 100, 100, None).pct(100), 100);
    }

    #[test]
    fn a_fired_timer_outranks_a_sooner_pending_one() {
        let timers = vec![
            timer(1, "tea", 0, 500, None),        // pending, soonest
            timer(2, "pasta", 0, 100, Some(100)), // fired
        ];
        let s = Status::from_timers(&timers, 200);
        assert_eq!(s.state, "firing");
        assert_eq!(
            s.name, "pasta",
            "the alarm is the headline, not the countdown"
        );
        assert_eq!((s.count, s.pending, s.firing), (2, 1, 1));
        assert_eq!(s.timers[0].name, "pasta", "firing sorts to the top");
    }

    #[test]
    fn a_private_timer_is_masked_for_the_bar_but_not_for_the_popup() {
        let s = Status::from_timers(&[private(1, "salary review", 0, 500, None)], 200);
        assert_eq!(s.name, MASKED_NAME, "the pill must not carry the real name");
        assert!(
            !s.tooltip.contains("salary"),
            "the tooltip needs no click, so it leaks first: {:?}",
            s.tooltip
        );
        assert_eq!(
            s.timers[0].name, "salary review",
            "the popup still shows the real name"
        );
        assert!(s.timers[0].private, "and marks the row as hidden");
        // Everything the bar derives from a deadline is untouched, so the pill
        // still counts down and still turns red.
        assert_eq!((s.count, s.pending, s.label.as_str()), (1, 1, "5:00"));
    }

    #[test]
    fn masking_survives_firing_and_mixed_lists() {
        let timers = vec![
            private(1, "salary review", 0, 100, Some(100)),
            timer(2, "tea", 0, 500, None),
        ];
        let s = Status::from_timers(&timers, 200);
        // The fired private timer is the headline, and its "fired N ago" line is
        // the one most likely to be forgotten.
        assert_eq!(s.state, "firing");
        assert_eq!(s.name, MASKED_NAME);
        assert!(!s.tooltip.contains("salary"), "{:?}", s.tooltip);
        // A public timer in the same list keeps its name.
        assert!(s.tooltip.contains("tea"), "{:?}", s.tooltip);
    }

    #[test]
    fn idle_says_so_and_stays_readable() {
        let s = Status::from_timers(&[], 200);
        assert_eq!((s.state.as_str(), s.count, s.pct), ("idle", 0, 0));
        assert!(s.tooltip.contains("no timers"));
        assert!(s.ok && !s.failed);
    }
}
