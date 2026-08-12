//! Argentine national holidays, and the working-day arithmetic on top of them.
//!
//! Two consumers, one model: `kimai` needs to know how many hours a month was
//! *supposed* to be, and `calendar-gui` needs to know what colour each day is.
//! Both questions come down to the same one — what does this date count as —
//! and answering it twice was how the pill and the calendar would end up
//! disagreeing about a Thursday.
//!
//! The hard part is not the fixed dates. It is that Argentina has two
//! categories the state declares and the *employer* decides:
//!
//! * *puentes turísticos*, declared by decree a year or two ahead and following
//!   no rule a program could derive; and
//! * *días no laborables* — Jueves Santo and the religious holidays — which the
//!   API does not list at all.
//!
//! So: fetch what is knowable from
//! [api.argentinadatos.com](https://api.argentinadatos.com/v1/feriados/2026),
//! cache it where losing it is not silent, and let a hand-edited
//! `overrides.toml` win over all of it. Editing that file takes effect on the
//! next poll — no rebuild, no restart — which is the whole reason it is a plain
//! file in `~/.config` rather than anything Nix manages.
//!
//! Nothing here ever fails a caller for want of data. A missing cache degrades
//! to the weekend rule and sets [`Coverage::degraded`], because a bar that
//! cannot draw its pill is worse than one that draws it slightly wrong and says
//! so.

pub mod cache;
pub mod calendar;
pub mod model;
pub mod month;
pub mod overrides;
pub mod source;

pub use calendar::{Calendar, Locations, Refresh};
pub use model::{Coverage, Day, DayKind, WorkWeek};
pub use month::{Cell, MonthView};
pub use overrides::Override;

#[cfg(test)]
mod tests;
