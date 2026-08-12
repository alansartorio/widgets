//! The chart itself: a pure render of one [`Chart`] onto a Cairo context.
//!
//! Nothing here holds state or talks to anything. The chart body stays a single
//! `DrawingArea` rather than a tree of widgets because every layer is positioned
//! in absolute pixels the daemon computed — sessions at real x positions, bars
//! pushed up by a fraction of the column height — which is more directly said in
//! Cairo than by wrestling GTK4's box layout into pixel-precise widths.
//!
//! This is also the one part of the window Relm4 cannot diff for us: a canvas has
//! no properties to compare, so [`crate::popup`] re-installs a closure over the
//! current state on every render pass. That works because this function is pure —
//! given the same chart it paints the same pixels, so re-running it is never
//! wrong, only wasted.

use claude_usage_core::models::Chart;
use relm4::gtk::cairo;

/// Chart body dimensions, matching the eww popup this replaced. The label strip
/// holds three rows: the bar labels, the session percentages under them, and the
/// weekly percentages under those.
pub const CHART_HEIGHT: i32 = 110;
const BARS_HEIGHT: i32 = 92;
pub const LABEL_HEIGHT: i32 = 48;
/// Bottom edge of a session rectangle, measured from the top of the bars.
const SESSION_BOTTOM: f64 = BARS_HEIGHT as f64 + 28.0;
/// Where each row of the label strip puts its baseline, measured from the top of
/// the strip. The weekly row sits lower in the day view, which has session
/// rectangles reaching into the strip for it to clear; the month view has none,
/// so its weekly label follows the bar labels directly.
const BAR_LABEL: f64 = 10.0;
const SESSION_LABEL: f64 = 22.0;
const WEEK_LABEL_DAY: f64 = 40.0;
const WEEK_LABEL_MONTH: f64 = 34.0;
/// How far a weekly rectangle reaches past its own label's baseline.
const WEEK_LABEL_ROOM: f64 = 6.0;

/// Which bar column an x coordinate falls in.
///
/// The columns are a uniform `bar_width` apart — the daemon sized them so
/// `bar_width × bars.len()` is the `chart_width` every layer agrees on — so this
/// is a division rather than a search. Returns `None` past the last column, which
/// is reachable: the drawing area is centred, and a click can land in the padding.
pub fn bar_at(chart: &Chart, x: f64) -> Option<usize> {
    if chart.bar_width <= 0 {
        return None;
    }
    let slot = (x / chart.bar_width as f64).floor();
    if slot < 0.0 {
        return None;
    }
    let slot = slot as usize;
    (slot < chart.bars.len()).then_some(slot)
}

/// Draw the chart: weekly rectangles at the back, session rectangles over them,
/// bars in front, labels under.
///
/// `hover` is the bar the pointer is over, which only changes anything in the
/// month view — the one where a bar is a link to somewhere else.
pub fn draw(chart: &Chart, hover: Option<usize>, ctx: &cairo::Context) {
    // Nord palette, normalized to 0.0–1.0.
    const NORD0: (f64, f64, f64) = (0.180, 0.204, 0.251); // weekly window bg
    const NORD3: (f64, f64, f64) = (0.298, 0.337, 0.416); // gap bar / session bg
    const NORD4: (f64, f64, f64) = (0.847, 0.871, 0.914); // bar label
    const NORD6: (f64, f64, f64) = (0.925, 0.937, 0.957); // session cost label
    const NORD7: (f64, f64, f64) = (0.561, 0.737, 0.733); // weekly border / label
    const NORD8: (f64, f64, f64) = (0.533, 0.753, 0.816); // session left border
    const NORD9: (f64, f64, f64) = (0.506, 0.631, 0.757); // bar fill (secondary)
    const NORD10: (f64, f64, f64) = (0.369, 0.506, 0.675); // banded session bg

    // The drawing area is sized to chart_width × (CHART_HEIGHT + LABEL_HEIGHT).
    // Bars occupy the top BARS_HEIGHT; labels sit in the strip below.
    let bars_top = 0.0_f64;
    let bars_bottom = BARS_HEIGHT as f64;
    let label_top = bars_bottom;

    // --- Weekly layer (at the back) ---
    // Absolute left edges, already clipped to the chart by the daemon. A darker
    // trough than the sessions that sit on top of it, keyed to the frost colour
    // its label is written in.
    let week_label = if chart.range == "day" {
        WEEK_LABEL_DAY
    } else {
        WEEK_LABEL_MONTH
    };
    let week_bottom = label_top + week_label + WEEK_LABEL_ROOM;
    for week in &chart.weeks {
        let x = week.x as f64;
        // The 2px on the right is the gutter before the next window — but only
        // when this one ended; a window cut off by the chart runs to the edge.
        let w = if week.closes {
            (week.width as f64 - 2.0).max(1.0)
        } else {
            week.width as f64
        };
        ctx.set_source_rgba(
            NORD0.0,
            NORD0.1,
            NORD0.2,
            if week.band { 0.75 } else { 0.5 },
        );
        round_rect_sides(
            ctx,
            x,
            bars_top,
            w,
            week_bottom,
            5.0,
            (week.opens, week.closes),
        );
        ctx.fill().ok();

        // Left border (nord7), which is what separates one week from the next —
        // and marks where this one opened, so a window already running when the
        // chart began doesn't get one.
        if week.opens {
            ctx.set_source_rgba(NORD7.0, NORD7.1, NORD7.2, 0.8);
            ctx.rectangle(x, bars_top, 2.0, week_bottom);
            ctx.fill().ok();
        }

        centred_text(
            ctx,
            &week.label,
            x,
            w,
            label_top + week_label,
            10.0,
            NORD7,
            1.0,
        );
    }

    // --- Sessions layer (over the weeks, behind the bars) ---
    // Drawn left-to-right from `lead_px`, each preceded by its gap.
    let mut x = chart.lead_px as f64;
    for session in &chart.sessions {
        // Idle gap before this window: leave it as background.
        x += session.gap as f64;
        let w = session.width as f64;
        // The rectangle runs past the bottom of the bars into the label strip,
        // and keeps its right-hand gutter only if the window ended in view.
        let rect_w = if session.closes { w - 2.0 } else { w };
        let session_bg = if session.band { NORD10 } else { NORD3 };
        ctx.set_source_rgba(session_bg.0, session_bg.1, session_bg.2, 0.55);
        let sides = (session.opens, session.closes);
        round_rect_sides(ctx, x, bars_top, rect_w, SESSION_BOTTOM, 5.0, sides);
        ctx.fill().ok();

        // Left border (nord8) — the mark of where the window opened, so one that
        // was already running when the day began doesn't get one.
        if session.opens {
            ctx.set_source_rgba(NORD8.0, NORD8.1, NORD8.2, 1.0);
            ctx.rectangle(x, bars_top, 2.0, SESSION_BOTTOM);
            ctx.fill().ok();
        }

        // Cost label, only if the rectangle is wide enough to hold it.
        if w >= 30.0 {
            centred_text(
                ctx,
                &session.label,
                x,
                w,
                label_top + SESSION_LABEL,
                10.0,
                NORD6,
                1.0,
            );
        }
        x += w;
    }

    // --- Bars layer (front) ---
    let bw = chart.bar_width as f64;
    for (i, bar) in chart.bars.iter().enumerate() {
        let bx = i as f64 * bw;

        // A gap gets no rectangle at all — the machine was off, which is a
        // different thing from an hour that cost nothing — just a dimmed label.
        if !bar.gap {
            // Filled bar pushed up from the bottom by (100 - pct)% of the height.
            let h = (bar.pct as f64 / 100.0) * BARS_HEIGHT as f64;
            // The bar slot has 2px margin on each side (matching eww
            // .usage-bar-slot).
            let is_hovered = chart.range == "month" && hover == Some(i) && !bar.date.is_empty();
            let fill = if is_hovered { NORD10 } else { NORD9 };
            ctx.set_source_rgba(fill.0, fill.1, fill.2, 1.0);
            round_rect(ctx, bx + 2.0, bars_bottom - h, bw - 4.0, h, 3.0);
            ctx.fill().ok();
        }

        // A gap's label is dimmed rather than dropped: the hour still happened,
        // and the axis would be missing a tick without it.
        let alpha = if bar.gap { 0.4 } else { 1.0 };
        centred_text(
            ctx,
            &bar.label,
            bx,
            bw,
            label_top + BAR_LABEL,
            9.0,
            NORD4,
            alpha,
        );
    }
}

/// Write `text` centred in a `width`-wide column starting at `x`, with its
/// baseline at `y`. Skips empty text, so callers don't each have to.
///
/// The horizontal centring needs the glyph extents, not the advance width, or
/// text with a left bearing sits a pixel or two off centre — which is visible
/// when a label has to line up with the rectangle drawn behind it.
#[allow(clippy::too_many_arguments)]
fn centred_text(
    ctx: &cairo::Context,
    text: &str,
    x: f64,
    width: f64,
    y: f64,
    size: f64,
    (r, g, b): (f64, f64, f64),
    alpha: f64,
) {
    if text.is_empty() {
        return;
    }
    ctx.set_source_rgba(r, g, b, alpha);
    ctx.set_font_size(size);
    if let Ok(ext) = ctx.text_extents(text) {
        ctx.move_to(x + (width - ext.width()) / 2.0 - ext.x_bearing(), y);
        ctx.show_text(text).ok();
    }
}

/// A rounded rectangle path (Cairo doesn't have one built in).
fn round_rect(ctx: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    round_rect_sides(ctx, x, y, w, h, r, (true, true));
}

/// The same path, but rounding only the `(left, right)` sides asked for. A
/// square side is how a window that runs off the chart is drawn: the corner would
/// say the span ended there, and it didn't.
fn round_rect_sides(
    ctx: &cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    (left, right): (bool, bool),
) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0);
    ctx.new_path();
    // Clockwise from the top-right. `arc` draws a line from the current point to
    // where the arc begins, and `line_to` without one behaves as a `move_to`, so
    // either opening step starts the path correctly.
    if right {
        ctx.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
        ctx.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    } else {
        ctx.line_to(x + w, y);
        ctx.line_to(x + w, y + h);
    }
    if left {
        ctx.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
        ctx.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    } else {
        ctx.line_to(x, y + h);
        ctx.line_to(x, y);
    }
    ctx.close_path();
}
