//! `hd-catalogue sheet`: the report as one static page, the most suspicious entries first.
//!
//! Each row shows the native picture (4x, nearest), the art the renderer would draw, the art
//! shrunk to the native size (4x, nearest: what the comparison scores see), who the entry is and
//! what the art says it is, and the flags and scores.

use super::verify::{Report, Verdict, read_indexed};
use super::{Category, Entry, read_catalogue, score};
use anyhow::Context;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

/// The rows beyond this many are left out of the page (the report has them all).
const MAX_ROWS: usize = 1200;
/// How much the native pictures are enlarged.
const ZOOM: usize = 4;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The picture of palette indexes `indexes` (`w` x `h`) in `palette`, enlarged `ZOOM` times with the
/// ground the page shows through index 0.
fn native_picture(indexes: &[u8], w: usize, palette: &[[u8; 3]; 16]) -> (usize, usize, Vec<u8>) {
    let h = indexes.len() / w;
    let mut rgba = vec![0_u8; w * ZOOM * h * ZOOM * 4];
    for y in 0..h * ZOOM {
        for x in 0..w * ZOOM {
            let index = indexes[(y / ZOOM) * w + x / ZOOM];
            if index != 0 {
                let c = palette[usize::from(index & 15)];
                rgba[(y * w * ZOOM + x) * 4..][..4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    }
    (w * ZOOM, h * ZOOM, rgba)
}

/// Writes an RGBA PNG, compressed (the page has thousands).
fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, ssd_hd::png::encode_rgba_compressed(w as u32, h as u32, rgba))
}

fn hex(text: &str) -> [u8; 3] {
    let t = text.trim_start_matches('#');
    let c = |at: usize| u8::from_str_radix(t.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0);
    [c(0), c(2), c(4)]
}

/// The art shrunk to the native size and enlarged like the native picture.
fn shrunk_picture(
    rgba: &[u8],
    aw: usize,
    ah: usize,
    w: usize,
    h: usize,
    mirrored: bool,
    upside_down: bool,
) -> Option<(usize, usize, Vec<u8>)> {
    let mut down = score::downsample(rgba, aw, ah, w, h)?;
    if mirrored {
        down = down.mirrored();
    }
    if upside_down {
        down = down.flipped();
    }
    let mut out = vec![0_u8; w * ZOOM * h * ZOOM * 4];
    for y in 0..h * ZOOM {
        for x in 0..w * ZOOM {
            let n = (y / ZOOM) * w + x / ZOOM;
            if down.covers(n) {
                let c = down.rgb[n];
                out[(y * w * ZOOM + x) * 4..][..4]
                    .copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
            }
        }
    }
    Some((w * ZOOM, h * ZOOM, out))
}

pub fn run(out: &Path, log: &mut dyn Write) -> anyhow::Result<()> {
    let catalogue = read_catalogue(out)?;
    let text = std::fs::read_to_string(out.join("report.json"))
        .context("reading report.json (run `hd-catalogue verify` first)")?;
    let report: Report = serde_json::from_str(&text).context("parsing report.json")?;
    let art_store = Path::new(&report.summary.art_store);
    let img = out.join("img");
    if img.exists() {
        std::fs::remove_dir_all(&img)?;
    }
    std::fs::create_dir_all(&img)?;
    let entry_of = |id: &str| -> Option<&Entry> {
        let base = id.strip_suffix("~v").unwrap_or(id);
        catalogue.entries.iter().find(|e| e.id == base)
    };
    let rows: Vec<&Verdict> =
        report.results.iter().filter(|v| !v.flags.is_empty()).take(MAX_ROWS).collect();
    let mut html = String::new();
    html.push_str(HEAD);
    let mut summary = String::new();
    for (flag, count) in &report.summary.by_flag {
        let _ = write!(summary, "<button data-flag=\"{flag}\">{flag} {count}</button> ");
    }
    let _ = write!(
        html,
        "<h1>HD art catalogue</h1><p>{} entries, {} checked; {} rows below (flagged, most suspicious first; the report has all).</p><p>Show only: {summary}<button data-flag=\"\">all</button> <label><input type=checkbox id=hu checked> hide unused poses</label></p><table>\
         <tr><th>#</th><th>native</th><th>HD art</th><th>HD shrunk</th><th>who</th><th>flags</th></tr>",
        report.summary.entries,
        report.summary.checked,
        rows.len(),
    );
    for (n, verdict) in rows.iter().enumerate() {
        let Some(entry) = entry_of(&verdict.id) else { continue };
        let palette: [[u8; 3]; 16] =
            std::array::from_fn(|i| entry.palette.get(i).map_or([0; 3], |t| hex(t)));
        let (_, _, mut indexes) = read_indexed(&out.join(&entry.png))?;
        let upside_down = verdict.id.ends_with("~v");
        if upside_down {
            indexes = indexes.chunks(entry.w).rev().flatten().copied().collect();
        }
        let safe = verdict.id.replace('~', "-");
        let (nw, nh, rgba) = native_picture(&indexes, entry.w, &palette);
        write_png(&img.join(format!("{safe}-native.png")), nw, nh, &rgba)?;
        let mut art_cell = String::from("<i>none</i>");
        let mut down_cell = String::new();
        if let Some(art) = &verdict.art {
            let bytes = std::fs::read(art_store.join(&art.file));
            if let Ok((aw, ah, rgba)) =
                bytes.map_err(|e| e.to_string()).and_then(|b| ssd_hd::png::read_rgba(&b))
            {
                let name = format!("{}.png", art.id);
                let target = img.join(&name);
                if !target.exists() {
                    write_png(&target, aw as usize, ah as usize, &rgba)?;
                }
                art_cell = format!(
                    "<img class=hd src=\"img/{name}\" title=\"{}\"><br><small>{}</small>",
                    escape(&art.id),
                    escape(&art.id)
                );
                let mirrored = verdict.found.ends_with("mirror");
                // What the renderer draws: the recoloured art when it recolours
                let drawn = art
                    .drawn
                    .as_ref()
                    .and_then(|path| std::fs::read(out.join(path)).ok())
                    .and_then(|b| ssd_hd::png::read_rgba(&b).ok());
                let (dw, dh, drgba) = drawn.as_ref().unwrap_or(&(aw, ah, rgba.clone())).clone();
                if let Some((sw, sh, small)) = shrunk_picture(
                    &drgba,
                    dw as usize,
                    dh as usize,
                    entry.w,
                    entry.h,
                    mirrored,
                    false,
                ) {
                    write_png(&img.join(format!("{safe}-down.png")), sw, sh, &small)?;
                    let note = if art.recoloured { "recoloured as drawn" } else { "as made" };
                    down_cell =
                        format!("<img src=\"img/{safe}-down.png\"><br><small>{note}</small>");
                }
            }
        }
        let who = match entry.category {
            Category::BattlePose => format!(
                "<b>{}</b> unit {} {}<br>poseset {} pose {}{}<br><small>{}</small>",
                escape(&verdict.identity.join(", ")),
                entry.units.first().copied().unwrap_or(0),
                escape(entry.unit_names.first().map_or("", String::as_str)),
                entry.poseset.unwrap_or(0),
                entry.pose.unwrap_or(0),
                if upside_down { " (upside down)" } else { "" },
                escape(&entry.origin),
            ),
            _ => format!(
                "<b>{}</b> {} {}/{}<br><small>units {:?}</small>",
                escape(&verdict.identity.join(", ")),
                escape(&entry.id),
                escape(entry.stream.as_deref().unwrap_or("")),
                entry.half.unwrap_or(0),
                entry.units,
            ),
        };
        let label = verdict.art.as_ref().map_or(String::new(), |art| {
            format!(
                "<br>art says: <i>{}</i><br>made for: {}",
                escape(art.description.as_deref().unwrap_or(&art.label)),
                escape(&art.made_for.join(", "))
            )
        });
        let used = if verdict.used {
            format!("used ({})", verdict.basis)
        } else {
            "unused: the game never shows this pose".to_owned()
        };
        let mut flags = String::new();
        for flag in &verdict.flags {
            let _ = write!(flags, "<span class=\"flag {flag}\">{flag}</span> ");
        }
        let mut scores = String::new();
        if let Some(v) = verdict.scores.iou {
            let _ = write!(scores, "silhouette {v:.2} ");
        }
        if let Some(v) = verdict.scores.colour_share {
            let _ = write!(
                scores,
                "colour {:.0}% (dist {:.0}) ",
                v * 100.0,
                verdict.scores.colour_distance.unwrap_or(0.0)
            );
        }
        if let Some(v) = verdict.scores.luma_correlation {
            let _ = write!(scores, "structure {v:.2} ");
        }
        let mut details = String::new();
        for detail in &verdict.details {
            let _ = write!(details, "<li>{}</li>", escape(detail));
        }
        let _ = write!(
            html,
            "<tr data-used=\"{}\" data-flags=\"{}\"><td>{}<br><small>{:.0}</small></td><td><img src=\"img/{safe}-native.png\"></td><td>{art_cell}</td><td>{down_cell}</td><td>{who}{label}<br><small>{used}</small></td><td>{flags}<br><small>{} via {}</small><ul>{details}</ul></td></tr>",
            verdict.used,
            verdict.flags.join(" "),
            n + 1,
            verdict.suspicion,
            escape(&scores),
            escape(&verdict.found),
        );
    }
    html.push_str("</table>");
    html.push_str(TAIL);
    std::fs::write(out.join("index.html"), html)?;
    writeln!(
        log,
        "hd-catalogue sheet: {} rows -> {}",
        rows.len(),
        out.join("index.html").display()
    )?;
    Ok(())
}

const HEAD: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>HD art catalogue</title><style>\
body{font:14px sans-serif;margin:12px;background:#1b1b1f;color:#ddd}table{border-collapse:collapse}\
td,th{border:1px solid #444;padding:4px;vertical-align:top}img{image-rendering:pixelated;background:#555}img.hd{image-rendering:auto;max-width:128px;max-height:128px}\
small{color:#999}ul{margin:2px;padding-left:16px}.flag{padding:1px 5px;border-radius:3px;background:#555}.identity{background:#a33}.shared{background:#a63}\
.shape{background:#36a}.colour{background:#7a3}.missing{background:#666}button{margin:2px}</style></head><body>";

const TAIL: &str = "<script>let cur='';function apply(){const hide=document.getElementById('hu').checked;document.querySelectorAll('tr[data-flags]').forEach(r=>{const ok=(!cur||r.dataset.flags.split(' ').includes(cur))&&!(hide&&r.dataset.used==='false');r.style.display=ok?'':'none'})}document.querySelectorAll('button[data-flag]').forEach(b=>b.onclick=()=>{cur=b.dataset.flag;apply()});document.getElementById('hu').onchange=apply;apply();</script></body></html>";
