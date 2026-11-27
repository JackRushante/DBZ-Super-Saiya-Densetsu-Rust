//! `hd-catalogue verify --check`: the report as a gate.
//!
//! Any flag on a used entry fails the check unless `tools/hd-catalogue-accept.toml` accepts that
//! flag on that entry with a reason. The list holds false alarms only (an entry checked by eye and
//! found fine); a real error is fixed in the art store, never accepted. An accepted entry that the
//! report no longer flags is stale and fails the check too, so the list does not outlive its reasons.

use super::verify::Report;
use anyhow::Context;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

/// How many open flags are listed one by one.
const LISTED: usize = 40;

#[derive(Debug, Deserialize)]
struct Accept {
    id: String,
    flags: Vec<String>,
    reason: String,
}

#[derive(Debug, Default, Deserialize)]
struct AcceptFile {
    #[serde(default)]
    accept: Vec<Accept>,
}

/// What the check found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Flags on used entries that nothing accepts: (entry, flag).
    pub open: Vec<(String, String)>,
    /// Accepted (entry, flag) pairs the report does not raise.
    pub stale: Vec<(String, String)>,
    /// Entries of the list without a reason or a flag.
    pub incomplete: Vec<String>,
}

impl Outcome {
    #[must_use]
    pub fn passes(&self) -> bool {
        self.open.is_empty() && self.stale.is_empty() && self.incomplete.is_empty()
    }
}

/// The accept list at `path` (empty when the file does not exist).
fn read_accept(path: &Path) -> anyhow::Result<AcceptFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AcceptFile::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Checks `report` against the accept list at `accept`.
///
/// # Errors
///
/// The list cannot be read.
pub fn check(report: &Report, accept: &Path) -> anyhow::Result<Outcome> {
    let file = read_accept(accept)?;
    Ok(judge(report, &file))
}

fn judge(report: &Report, file: &AcceptFile) -> Outcome {
    let mut outcome = Outcome::default();
    let mut accepted: BTreeSet<(String, String)> = BTreeSet::new();
    for entry in &file.accept {
        if entry.reason.trim().is_empty() || entry.flags.is_empty() {
            outcome.incomplete.push(entry.id.clone());
        }
        for flag in &entry.flags {
            accepted.insert((entry.id.clone(), flag.clone()));
        }
    }
    let mut raised: BTreeSet<(String, String)> = BTreeSet::new();
    for verdict in report.results.iter().filter(|v| v.used) {
        for flag in &verdict.flags {
            let pair = (verdict.id.clone(), flag.clone());
            if !accepted.contains(&pair) {
                outcome.open.push(pair.clone());
            }
            raised.insert(pair);
        }
    }
    outcome.stale = accepted.difference(&raised).cloned().collect();
    outcome.open.sort();
    outcome
}

/// Prints the outcome: counts per flag, the first open entries, the stale and incomplete ones.
pub fn print(outcome: &Outcome, accepted: usize, log: &mut dyn Write) -> std::io::Result<()> {
    let mut by_flag: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, flag) in &outcome.open {
        *by_flag.entry(flag.as_str()).or_default() += 1;
    }
    writeln!(
        log,
        "hd-catalogue check: {} open flags on used entries {by_flag:?}, {accepted} accepted, {} stale, {} incomplete -> {}",
        outcome.open.len(),
        outcome.stale.len(),
        outcome.incomplete.len(),
        if outcome.passes() { "ok" } else { "FAILED" }
    )?;
    for (id, flag) in outcome.open.iter().take(LISTED) {
        writeln!(log, "  {id}: {flag}")?;
    }
    if outcome.open.len() > LISTED {
        writeln!(log, "  ... and {} more (report.json has all)", outcome.open.len() - LISTED)?;
    }
    for (id, flag) in &outcome.stale {
        writeln!(log, "  stale accept: {id} {flag} is not flagged any more: remove it")?;
    }
    for id in &outcome.incomplete {
        writeln!(log, "  incomplete accept: {id} needs flags and a reason")?;
    }
    Ok(())
}

/// The (entry, flag) pairs the list at `path` accepts.
#[must_use]
pub fn accepted_pairs(path: &Path) -> BTreeSet<(String, String)> {
    let mut pairs = BTreeSet::new();
    for entry in read_accept(path).map(|f| f.accept).unwrap_or_default() {
        for flag in entry.flags {
            pairs.insert((entry.id.clone(), flag));
        }
    }
    pairs
}

/// How many (entry, flag) pairs the list at `path` accepts.
#[must_use]
pub fn accepted_count(path: &Path) -> usize {
    read_accept(path).map_or(0, |file| file.accept.iter().map(|a| a.flags.len()).sum())
}

#[cfg(test)]
mod tests {
    use super::super::Category;
    use super::super::verify::{Scores, Summary, Verdict};
    use super::*;

    fn verdict(id: &str, used: bool, flags: &[&str]) -> Verdict {
        Verdict {
            id: id.to_owned(),
            category: Category::BattlePose,
            used,
            basis: String::new(),
            identity: Vec::new(),
            flags: flags.iter().map(|f| (*f).to_owned()).collect(),
            suspicion: 0.0,
            found: "exact".to_owned(),
            art: None,
            scores: Scores::default(),
            details: Vec::new(),
        }
    }

    fn report(results: Vec<Verdict>) -> Report {
        Report { summary: Summary::default(), results }
    }

    #[test]
    fn a_flag_on_a_used_entry_fails_unless_it_is_accepted_and_unused_entries_do_not_count() {
        let report = report(vec![
            verdict("a", true, &["colour", "shape"]),
            verdict("b", false, &["missing"]),
            verdict("c", true, &[]),
        ]);
        let file: AcceptFile = toml::from_str(
            "[[accept]]\nid = \"a\"\nflags = [\"colour\"]\nreason = \"the art is fine, its shading differs\"\n",
        )
        .unwrap();
        let outcome = judge(&report, &file);
        assert_eq!(outcome.open, [("a".to_owned(), "shape".to_owned())]);
        assert!(!outcome.passes());
        let file: AcceptFile = toml::from_str(
            "[[accept]]\nid = \"a\"\nflags = [\"colour\", \"shape\"]\nreason = \"checked by eye\"\n",
        )
        .unwrap();
        assert!(judge(&report, &file).passes());
    }

    #[test]
    fn an_accepted_flag_that_is_not_raised_and_an_entry_without_a_reason_fail() {
        let report = report(vec![verdict("a", true, &[])]);
        let file: AcceptFile =
            toml::from_str("[[accept]]\nid = \"a\"\nflags = [\"colour\"]\nreason = \"\"\n")
                .unwrap();
        let outcome = judge(&report, &file);
        assert_eq!(outcome.stale, [("a".to_owned(), "colour".to_owned())]);
        assert_eq!(outcome.incomplete, ["a"]);
        assert!(!outcome.passes());
    }
}
