//! `hd-catalogue redo`: the art that has to be made again, from `report.json`.
//!
//! An entry is a real error when the game shows it, and its art is another character's or another
//! look of the same one, is described as another look, has the wrong colours or the wrong shape.
//! A portrait whose description does not name who it shows is one too (the description was
//! guessed from the pixels; that is how the Namekian child was drawn as an adult). A flag on a name
//! alone (`label`, `shared`) is not: the picture is right. Entries with no art at all are listed
//! apart as `missing`. Each identity carries a plain description of the character to draw from
//! `tools/hd-identity.toml` (`describe`).

use super::identity::Identity;
use super::verify::{Report, Verdict};
use super::{Category, Entry, read_catalogue};
use anyhow::Context;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

/// One entry to draw again.
#[derive(Debug, Serialize)]
pub struct Item {
    /// The catalogue id (`~v` for the pose drawn upside down).
    pub id: String,
    pub category: Category,
    pub unit: Option<u8>,
    pub poseset: Option<u8>,
    pub pose: Option<usize>,
    /// The native picture, absolute.
    pub native_png: String,
    /// The art it is drawn from now (its id and its file in the art store), when it has some.
    pub art_id: Option<String>,
    pub art_file: Option<String>,
    pub flags: Vec<String>,
    /// `certain`, or `check` for a portrait whose description does not name who it shows.
    pub confidence: &'static str,
    pub why: Vec<String>,
}

/// The entries of one identity (a unit and its look).
#[derive(Debug, Serialize)]
pub struct Group {
    pub identity: String,
    /// Plain words for what to draw; empty when the table has none yet.
    pub description: String,
    pub wrong: Vec<Item>,
    pub missing: Vec<Item>,
}

#[derive(Debug, Serialize)]
pub struct Redo {
    pub art_store: String,
    pub identities: Vec<Group>,
}

/// Whether `details` say the picture itself is wrong, not only its name.
fn picture_errors(verdict: &Verdict, entry: &Entry) -> Vec<String> {
    let mut why = Vec::new();
    for detail in &verdict.details {
        let wrong_art = detail.contains("is drawn from art made for");
        let look = detail.contains("does not describe the") || detail.contains("which ");
        let guessed = entry.category != Category::BattlePose
            && detail.contains("the label does not name")
            && verdict.flags.iter().any(|f| f == "identity");
        if wrong_art || look || guessed {
            why.push(detail.clone());
        }
    }
    for flag in ["colour", "shape"] {
        if verdict.flags.iter().any(|f| f == flag)
            && let Some(detail) = verdict.details.iter().find(|d| match flag {
                "colour" => d.contains("palette indexes"),
                _ => d.contains("silhouette") || d.contains("follow the picture"),
            })
        {
            why.push(detail.clone());
        }
    }
    why
}

/// Builds the list.
///
/// # Errors
///
/// The report or the catalogue cannot be read.
pub fn build(
    out: &Path,
    identity: &Identity,
    accepted: &BTreeSet<(String, String)>,
) -> anyhow::Result<Redo> {
    let catalogue = read_catalogue(out)?;
    let text = std::fs::read_to_string(out.join("report.json"))
        .context("reading report.json (run `hd-catalogue verify` first)")?;
    let report: Report = serde_json::from_str(&text).context("parsing report.json")?;
    let entries: BTreeMap<&str, &Entry> =
        catalogue.entries.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for verdict in report.results.iter().filter(|v| v.used && !v.flags.is_empty()) {
        let mut verdict = verdict.clone();
        verdict.flags.retain(|flag| !accepted.contains(&(verdict.id.clone(), flag.clone())));
        let verdict = &verdict;
        if verdict.flags.is_empty() {
            continue;
        }
        let base = verdict.id.strip_suffix("~v").unwrap_or(&verdict.id);
        let Some(entry) = entries.get(base) else { continue };
        let missing = verdict.flags.iter().any(|f| f == "missing");
        let why = if missing {
            vec!["no art for this pose".to_owned()]
        } else {
            picture_errors(verdict, entry)
        };
        if why.is_empty() {
            continue;
        }
        let slug = verdict.identity.first().cloned().unwrap_or_else(|| "unknown".to_owned());
        let group = groups.entry(slug.clone()).or_insert_with(|| Group {
            description: identity.describe(&slug).unwrap_or_default().to_owned(),
            identity: slug,
            wrong: Vec::new(),
            missing: Vec::new(),
        });
        let item = Item {
            id: verdict.id.clone(),
            category: entry.category,
            unit: entry.units.first().copied().filter(|_| entry.category == Category::BattlePose),
            poseset: entry.poseset,
            pose: entry.pose,
            native_png: out.join(&entry.png).display().to_string(),
            art_id: verdict.art.as_ref().map(|a| a.id.clone()),
            art_file: verdict.art.as_ref().map(|a| a.file.clone()),
            flags: verdict.flags.clone(),
            confidence: if entry.category == Category::BattlePose
                || !verdict.flags.iter().any(|f| f == "identity")
            {
                "certain"
            } else {
                "check"
            },
            why,
        };
        if missing {
            group.missing.push(item);
        } else {
            group.wrong.push(item);
        }
    }
    let mut identities: Vec<Group> = groups.into_values().collect();
    identities.sort_by(|a, b| {
        b.wrong.len().cmp(&a.wrong.len()).then_with(|| a.identity.cmp(&b.identity))
    });
    Ok(Redo { art_store: report.summary.art_store, identities })
}

pub fn run(
    out: &Path,
    identity_path: &Path,
    accept_path: &Path,
    log: &mut dyn Write,
) -> anyhow::Result<()> {
    let identity = Identity::read(identity_path)?;
    let redo = build(out, &identity, &super::gate::accepted_pairs(accept_path))?;
    std::fs::write(out.join("redo.json"), serde_json::to_string_pretty(&redo)?)?;
    let wrong: usize = redo.identities.iter().map(|g| g.wrong.len()).sum();
    let missing: usize = redo.identities.iter().map(|g| g.missing.len()).sum();
    let undescribed: Vec<&str> = redo
        .identities
        .iter()
        .filter(|g| !g.wrong.is_empty() && g.description.is_empty())
        .map(|g| g.identity.as_str())
        .collect();
    writeln!(
        log,
        "hd-catalogue redo: {wrong} entries of wrong art in {} identities, {missing} used entries without art -> {}",
        redo.identities.iter().filter(|g| !g.wrong.is_empty()).count(),
        out.join("redo.json").display()
    )?;
    for group in redo.identities.iter().filter(|g| !g.wrong.is_empty()) {
        writeln!(
            log,
            "  {:<18} {:>3} wrong, {:>3} missing",
            group.identity,
            group.wrong.len(),
            group.missing.len()
        )?;
    }
    if !undescribed.is_empty() {
        writeln!(log, "  no description yet for: {}", undescribed.join(", "))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::verify::{ArtRef, Scores, Summary};
    use super::super::{Catalogue, Entry};
    use super::*;

    fn entry(id: &str, unit: u8) -> Entry {
        Entry {
            id: id.to_owned(),
            category: Category::BattlePose,
            units: vec![unit],
            unit_names: Vec::new(),
            poseset: Some(10),
            pose: Some(0),
            stream: None,
            half: None,
            object: None,
            shown_by: Vec::new(),
            w: 8,
            h: 8,
            key: None,
            mirror_key: None,
            vflip_keys: None,
            cluster_keys: Vec::new(),
            origin: String::new(),
            palette: Vec::new(),
            png: format!("native/{id}.png"),
            note: None,
            used: true,
            evidence: Vec::new(),
            basis: "script".to_owned(),
        }
    }

    fn verdict(id: &str, slug: &str, flags: &[&str], details: &[&str]) -> Verdict {
        Verdict {
            id: id.to_owned(),
            category: Category::BattlePose,
            used: true,
            basis: "script".to_owned(),
            identity: vec![slug.to_owned()],
            flags: flags.iter().map(|f| (*f).to_owned()).collect(),
            suspicion: 0.0,
            found: "exact".to_owned(),
            art: Some(ArtRef {
                id: "art-001".to_owned(),
                file: "class2/art-001.png".to_owned(),
                fighter: None,
                description: None,
                label: String::new(),
                made_for: Vec::new(),
                palette_distance: None,
                recoloured: false,
                drawn: None,
            }),
            scores: Scores::default(),
            details: details.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    /// A wrong look is to redo; a wrong name, an accepted flag and an unflagged entry are not; an entry with
    /// no art is listed apart.
    #[test]
    fn only_wrong_pictures_are_to_redo_and_missing_ones_are_apart() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let catalogue = Catalogue {
            entries: ["a", "b", "c", "d", "e"]
                .iter()
                .enumerate()
                .map(|(n, id)| entry(id, n as u8))
                .collect(),
            problems: Vec::new(),
        };
        std::fs::write(out.join("catalogue.json"), serde_json::to_string(&catalogue).unwrap())
            .unwrap();
        let mut gone = verdict("e", "x", &["missing"], &[]);
        gone.art = None;
        let report = Report {
            summary: Summary::default(),
            results: vec![
                verdict(
                    "a",
                    "x",
                    &["identity"],
                    &[
                        "x is drawn from art made for y, another look of the same character (art-001)",
                    ],
                ),
                verdict(
                    "b",
                    "x",
                    &["label"],
                    &["art-001 says it is made for z, but the name is wrong"],
                ),
                verdict(
                    "c",
                    "x",
                    &["colour"],
                    &["the art as drawn is 120 from the colours of its palette indexes on average"],
                ),
                verdict("d", "x", &[], &[]),
                gone,
            ],
        };
        std::fs::write(out.join("report.json"), serde_json::to_string(&report).unwrap()).unwrap();
        let none = BTreeSet::new();
        let redo = build(out, &Identity::default(), &none).unwrap();
        assert_eq!(redo.identities.len(), 1);
        let group = &redo.identities[0];
        assert_eq!(group.wrong.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        assert_eq!(group.missing.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["e"]);
        let accepted: BTreeSet<(String, String)> = [("c".to_owned(), "colour".to_owned())].into();
        let redo = build(out, &Identity::default(), &accepted).unwrap();
        assert_eq!(redo.identities[0].wrong.len(), 1);
    }
}
