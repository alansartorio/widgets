//! Where the feriados come from.
//!
//! <https://api.argentinadatos.com/v1/feriados/{year}> — a JSON array of
//! `{fecha, tipo, nombre}`, one entry per date, with `tipo` observed as
//! `inamovible`, `trasladable` and `puente`. 2026 has nineteen.
//!
//! Fetched rather than computed, and that is a deliberate trade. Most of the
//! calendar *is* computable — fixed dates, Carnaval and Viernes Santo from
//! Easter, the trasladable rules — but the *puentes turísticos* are declared by
//! decree a year or two ahead and follow no rule at all. A ruleset would be
//! right about fifteen days and silently wrong about three, every year, forever.
//!
//! What the API does not have is the *días no laborables* (Jueves Santo, the
//! religious holidays) — those are what the override file is for.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const ENDPOINT: &str = "https://api.argentinadatos.com/v1/feriados";
/// Well short of anything a caller waits on interactively. A refresh that
/// cannot finish in this is a refresh that should be skipped in favour of the
/// cache.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// One entry as the API sends it, kept verbatim so the on-disk cache is the
/// response and a future field costs nothing to start reading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiDay {
    /// `YYYY-MM-DD`.
    pub fecha: String,
    /// `inamovible` | `trasladable` | `puente`, so far.
    pub tipo: String,
    pub nombre: String,
}

/// Ask the API for one year.
///
/// No agent to pass in any more: `widgets-http` makes a handle per request, so
/// there is nothing for a caller to hold on to between years.
pub fn fetch_year(year: i32) -> Result<Vec<ApiDay>> {
    let url = format!("{ENDPOINT}/{year}");
    let body = widgets_http::Request::get(&url)
        .accept_json()
        .timeout(REQUEST_TIMEOUT)
        .send()
        .with_context(|| format!("fetching {url}"))?
        .success()
        .context("reading the feriados response")?;

    let days: Vec<ApiDay> =
        serde_json::from_str(&body).context("the feriados response is not the expected JSON")?;

    // An empty array parses fine and would overwrite a good cache with nothing.
    // A year with no feriados does not exist in Argentina, so this is a broken
    // upstream rather than a real answer.
    anyhow::ensure!(!days.is_empty(), "the API returned no feriados for {year}");
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slice of the real 2026 payload, so the parser is tested against the
    /// bytes the API actually sends rather than against what this file assumes.
    const SAMPLE: &str = r#"[
      {"fecha": "2026-01-01", "tipo": "inamovible", "nombre": "Año nuevo"},
      {"fecha": "2026-03-23", "tipo": "puente", "nombre": "Puente turístico no laborable"},
      {"fecha": "2026-06-15", "tipo": "trasladable",
       "nombre": "Paso a la Inmortalidad del General Martín Güemes (17/6)"}
    ]"#;

    #[test]
    fn the_real_payload_parses() {
        let days: Vec<ApiDay> = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(days.len(), 3);
        assert_eq!(days[0].fecha, "2026-01-01");
        assert_eq!(days[1].tipo, "puente");
        assert!(days[2].nombre.starts_with("Paso a la Inmortalidad"));
    }
}
