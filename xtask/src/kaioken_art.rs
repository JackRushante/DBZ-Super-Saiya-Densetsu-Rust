//! `kaioken-art <out pack dir>`: copies the Super Saiyan Kaioken's pose variants into a pack folder, `<out pack dir>/art/hd/variants/`
//! (`examples/ssj-kaioken`, `docs/mods/art.md`).
//!
//! The 27 pictures are the battle poses of the Super Saiyan Goku (`battle-goku-ssj-000` to `026`) redrawn with the Kaioken's aura,
//! made locally from the installed HD art: they are read from `.work/m10/kk-art/variants/` of the repository. They are derived from
//! the game, so the pack's `art/` folder is never tracked (`.gitignore`), and the pack loads without it: its variants are then not
//! drawn.

use anyhow::{Context, bail};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The number of pictures a variant set holds (one per Super Saiyan Goku pose the aura is drawn over).
const PICTURES: usize = 27;
/// The prefix of the pictures' names.
const PREFIX: &str = "battle-goku-ssj-";

/// The command: `kaioken-art <out pack dir>`.
///
/// # Errors
///
/// The variant set is missing or not of [`PICTURES`] pictures, or a picture cannot be copied.
pub fn command(args: &[&str]) -> anyhow::Result<ExitCode> {
    let [out] = args else {
        eprintln!("kaioken-art: takes one argument, the pack folder (`examples/ssj-kaioken`)");
        return Ok(crate::usage());
    };
    let source = crate::repo_root()?.join(".work").join("m10").join("kk-art").join("variants");
    let copied = copy_variants(&source, Path::new(out))?;
    println!(
        "kaioken-art: {copied} pictures written to {}",
        Path::new(out).join("art/hd/variants").display()
    );
    Ok(ExitCode::SUCCESS)
}

/// Copies the variant pictures of `source` into `pack/art/hd/variants/`; returns how many.
///
/// # Errors
///
/// `source` does not hold [`PICTURES`] pictures named `battle-goku-ssj-NNN.png`, or one of them cannot be copied.
pub fn copy_variants(source: &Path, pack: &Path) -> anyhow::Result<usize> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(source)
        .with_context(|| {
            format!(
                "reading {}: the variants are made locally in .work/m10/kk-art",
                source.display()
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                name.starts_with(PREFIX)
                    && Path::new(name).extension().is_some_and(|ext| ext == "png")
            })
        })
        .collect();
    names.sort();
    if names.len() != PICTURES {
        bail!(
            "{} holds {} pictures of {PREFIX}NNN.png, not {PICTURES}",
            source.display(),
            names.len()
        );
    }
    let target = pack.join("art").join("hd").join("variants");
    std::fs::create_dir_all(&target).with_context(|| format!("creating {}", target.display()))?;
    for path in &names {
        let file = path.file_name().context("a picture without a name")?;
        std::fs::copy(path, target.join(file))
            .with_context(|| format!("copying {}", path.display()))?;
    }
    Ok(names.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variant_set_of_the_wrong_size_is_refused_and_a_right_one_is_copied() {
        let dir = std::env::temp_dir().join(format!("kaioken-art-{}", std::process::id()));
        let source = dir.join("variants");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("variants.json"), b"{}").unwrap();
        for n in 0..PICTURES - 1 {
            std::fs::write(source.join(format!("{PREFIX}{n:03}.png")), b"png").unwrap();
        }
        assert!(copy_variants(&source, &dir.join("pack")).is_err(), "26 pictures are not 27");
        std::fs::write(source.join(format!("{PREFIX}{:03}.png", PICTURES - 1)), b"png").unwrap();
        assert_eq!(copy_variants(&source, &dir.join("pack")).unwrap(), PICTURES);
        assert!(dir.join("pack/art/hd/variants/battle-goku-ssj-026.png").is_file());
        assert!(!dir.join("pack/art/hd/variants/variants.json").exists(), "only the pictures");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
