//! The file you edit at 08:55 when it turns out today is not being worked.
//!
//! It exists because two categories in the Argentine calendar are the
//! employer's call rather than the state's — *puentes turísticos* and *días no
//! laborables* — and because the API knows nothing about the second kind at
//! all. Whatever is in here wins over whatever the API said, per date.
//!
//! ```toml
//! # kind: holiday | non-working | bridge | work
//! [days]
//! "2026-04-01" = { kind = "non-working", name = "Jueves Santo" }
//! "2026-03-23" = { kind = "work" }                  # el puente se trabaja
//! ```
//!
//! Read with `toml_edit` and written with it too, so `kimai holidays set` adds
//! a row to a file a person wrote without eating the comments above it. A
//! parse-then-reserialise round trip through a plain `toml` would.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::model::DayKind;

/// One line of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Override {
    pub kind: DayKind,
    pub name: Option<String>,
}

/// Every date the file has an opinion about.
pub type Overrides = BTreeMap<NaiveDate, Override>;

/// The seed written when no file exists, which is also the documentation:
/// nobody reads a man page for a file they are about to edit, but they do read
/// the file.
pub const TEMPLATE: &str = r#"# Días que la API de feriados no conoce, o que decidimos distinto.
#
# Se lee en cada poll, así que un cambio acá tiene efecto en el próximo tick:
# no hace falta rebuild ni reiniciar nada.
#
# kind:
#   holiday      feriado — no se trabaja, no suma a lo esperado
#   non-working  día no laborable (Jueves Santo y compañía) — no se trabaja
#   bridge       puente turístico
#   work         se trabaja, aunque el feriado nacional diga otra cosa
#
# [days]
# "2026-04-01" = { kind = "non-working", name = "Jueves Santo" }
# "2026-03-23" = { kind = "work" }                  # el puente se trabaja

[days]
"#;

/// Read the file. A missing one is an empty set, not an error — the common case
/// is a machine where nothing has ever needed overriding.
pub fn load(path: &Path) -> Result<Overrides> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Ok(Overrides::new());
    };
    parse(&raw).with_context(|| format!("reading {}", path.display()))
}

pub fn parse(raw: &str) -> Result<Overrides> {
    let doc: DocumentMut = raw.parse().context("this is not valid TOML")?;
    let mut out = Overrides::new();

    let Some(days) = doc.get("days").and_then(Item::as_table_like) else {
        return Ok(out);
    };

    for (key, item) in days.iter() {
        let date: NaiveDate = key
            .parse()
            .with_context(|| format!("{key:?} is not a YYYY-MM-DD date"))?;

        // Two spellings accepted: the inline table with a name, and a bare
        // string for the common case where the kind is all there is to say.
        let (kind_raw, name) = match item {
            Item::Value(v) if v.is_str() => (v.as_str().unwrap_or_default().to_string(), None),
            _ => {
                let table = item
                    .as_table_like()
                    .with_context(|| format!("{key:?} should be a string or a table"))?;
                let kind = table
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .with_context(|| format!("{key:?} has no `kind`"))?
                    .to_string();
                let name = table
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(str::to_string);
                (kind, name)
            }
        };

        let kind = DayKind::parse(&kind_raw)
            .with_context(|| format!("{key:?} has an unknown kind {kind_raw:?}"))?;
        out.insert(date, Override { kind, name });
    }

    Ok(out)
}

/// Add or replace one date, leaving every comment and every other row alone.
pub fn set(path: &Path, date: NaiveDate, kind: DayKind, name: Option<&str>) -> Result<()> {
    edit(path, |days| {
        let mut entry = toml_edit::InlineTable::new();
        entry.insert("kind", kind.as_str().into());
        if let Some(name) = name {
            entry.insert("name", name.into());
        }
        days.insert(&date.to_string(), value(entry));
    })
}

/// Drop one date. Removing a date that was never there is a no-op, not an
/// error — `unset` is what you reach for when you are not sure.
pub fn unset(path: &Path, date: NaiveDate) -> Result<()> {
    edit(path, |days| {
        days.remove(&date.to_string());
    })
}

fn edit(path: &Path, change: impl FnOnce(&mut Table)) -> Result<()> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| TEMPLATE.to_string());
    let mut doc: DocumentMut = raw
        .parse()
        .with_context(|| format!("{} is not valid TOML", path.display()))?;

    let days = doc
        .entry("days")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .context("`days` is not a table")?;
    change(days);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Same temp-and-rename as the cache: this is a file the user is likely to
    // have open in an editor, and a truncated write would lose their comments
    // as well as ours.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_spellings_parse() {
        let got = parse(
            r#"
            [days]
            "2026-04-01" = { kind = "non-working", name = "Jueves Santo" }
            "2026-03-23" = "work"
            "#,
        )
        .unwrap();

        assert_eq!(
            got[&NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()],
            Override {
                kind: DayKind::NonWorking,
                name: Some("Jueves Santo".into())
            }
        );
        assert_eq!(
            got[&NaiveDate::from_ymd_opt(2026, 3, 23).unwrap()].kind,
            DayKind::ForcedWork
        );
    }

    #[test]
    fn an_absent_days_table_is_empty_rather_than_an_error() {
        assert!(parse("# nada por acá\n").unwrap().is_empty());
    }

    #[test]
    fn a_bad_kind_names_the_date() {
        let err = parse("[days]\n\"2026-01-01\" = \"jueves\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("2026-01-01"), "{err}");
    }

    #[test]
    fn setting_a_date_keeps_the_comments() {
        let dir = tempdir();
        let path = dir.join("overrides.toml");
        std::fs::write(&path, "# no me borres\n[days]\n").unwrap();

        set(
            &path,
            NaiveDate::from_ymd_opt(2026, 5, 8).unwrap(),
            DayKind::Holiday,
            Some("Puente"),
        )
        .unwrap();

        let back = std::fs::read_to_string(&path).unwrap();
        assert!(back.contains("# no me borres"), "{back}");
        assert!(back.contains("2026-05-08"), "{back}");

        unset(&path, NaiveDate::from_ymd_opt(2026, 5, 8).unwrap()).unwrap();
        let back = std::fs::read_to_string(&path).unwrap();
        assert!(back.contains("# no me borres"), "{back}");
        assert!(!back.contains("2026-05-08"), "{back}");
    }

    /// A scratch directory, without pulling in a crate for it.
    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "holidays-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
