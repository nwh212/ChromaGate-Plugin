//! ChromaGate: a Chromaforge plug-in that finds the picture area of scanned film frames and
//! sets it as each photo's crop. The crop is an ordinary one: undo it, or adjust it in the crop
//! tool when the detection is off.
//!
//! Needs the host features `pixels` (to look at each photo), `crop` (to set its crop) and
//! `virtualCopy` (to give each extra exposure of a half-frame scan a photo of its own).

pub mod detect;
pub mod split;

use detect::{FORMATS, Format};
use lcc_plugin_sdk::serde_json::{Value, json};
use lcc_plugin_sdk::{Request, export_plugin, host};

/// The manifest, with the version from Cargo.toml and the pictures from `brand/` filled in by
/// [`manifest`]. The id never changes: an install with another id is another plug-in.
const TEMPLATE: &str = r#"{
  "id": "io.github.nwh212.chromagate",
  "name": "ChromaGate",
  "version": "@version",
  "description": "Finds the film gate in scanned frames (negatives, converted negatives, slides in their mounts, scans on white or black) and sets a level crop inside it, which you can then adjust in the crop tool.",
  "author": "nwh212",
  "homepage": "https://github.com/nwh212/ChromaGate-Plugin",
  "repository": "https://github.com/nwh212/ChromaGate-Plugin",
  "changelog": "https://github.com/nwh212/ChromaGate-Plugin/blob/main/CHANGELOG.md",
  "license": "MIT",
  "icon": "@icon",
  "header": {"dark": "@header_dark", "light": "@header_light"},
  "requires": ["pixels", "crop", "virtualCopy"],
  "permissions": {"pixels": true, "crop": true, "virtualCopy": true},
  "commands": [
    {
      "id": "autocrop",
      "label": "Auto-Crop Film Frames…",
      "dialog": [
        {"key": "format", "label": "Frame format", "type": "choice",
         "options": ["Auto", "35mm / 6×9 (3:2)", "6×4.5 / half-frame (4:3)", "6×6 (1:1)", "6×7 / 4×5", "Any shape"], "default": "Auto"},
        {"key": "inset", "label": "Trim inside the frame edge (% of the side)", "type": "number", "min": 0, "max": 5, "default": 0.4}
      ]
    },
    {
      "id": "split",
      "label": "Split Half-Frame Scans…",
      "dialog": [
        {"key": "count", "label": "Exposures in each scan", "type": "choice",
         "options": ["2 (half-frame)", "3", "4"], "default": "2 (half-frame)"},
        {"key": "keep", "label": "The original keeps", "type": "choice",
         "options": ["The first exposure (left or top)", "The last exposure (right or bottom)"], "default": "The first exposure (left or top)"},
        {"key": "gate", "label": "Gate shape", "type": "choice",
         "options": ["35mm (3:2)", "Auto", "Any shape"], "default": "35mm (3:2)"},
        {"key": "inset", "label": "Trim inside each exposure (% of the side)", "type": "number", "min": 0, "max": 5, "default": 1.0}
      ]
    }
  ]
}"#;

/// `s` as a JSON string literal.
fn json_string(s: &str) -> String {
    Value::from(s).to_string()
}

/// The manifest the host reads. `icon` (a square SVG) and `header` (the wordmark, for dark and
/// light themes) are shown by hosts that read them, in place of the icon drawn from the id and
/// the name; others ignore them.
pub fn manifest() -> String {
    TEMPLATE
        .replace("\"@version\"", &json_string(env!("CARGO_PKG_VERSION")))
        .replace("\"@icon\"", &json_string(include_str!("../brand/icon.svg")))
        .replace(
            "\"@header_dark\"",
            &json_string(include_str!("../brand/logo-dark.svg")),
        )
        .replace(
            "\"@header_light\"",
            &json_string(include_str!("../brand/logo-light.svg")),
        )
}

/// Long edge of the pixels each photo is looked at in: an edge lands within 0.2 % (half the
/// default trim), and many photos fit in one call's instruction budget.
const LOOK_EDGE: u32 = 512;

/// The dialog's "Frame format" choice.
fn format_arg(v: &Value) -> Format {
    match v.as_str() {
        None | Some("Auto") => Format::Auto,
        Some("Any shape") => Format::Any,
        Some(name) => FORMATS
            .iter()
            .position(|(n, _, _)| *n == name)
            .map_or(Format::Auto, Format::Only),
    }
}

/// What happened to one photo.
enum Outcome {
    Cropped(detect::Detection),
    NoFrame,
    Failed(String),
}

fn crop_one(photo: &str, opts: &detect::Options) -> Outcome {
    let px = match host::photo_pixels(photo, LOOK_EDGE) {
        Ok(px) => px,
        Err(e) => return Outcome::Failed(e),
    };
    let Some(d) = detect::detect(&px.rgb, px.width, px.height, opts) else {
        return Outcome::NoFrame;
    };
    match host::set_crop(photo, d.rect, Some(d.angle)) {
        Ok(_) => Outcome::Cropped(d),
        Err(e) => Outcome::Failed(e),
    }
}

/// Most photos one run crops. A call has an instruction budget (2G by default), and each photo
/// takes a share of it: about 100M at [`LOOK_EDGE`], 185M when the frame is tilted (it's measured
/// again, straightened), 280M for a tilted square frame. A larger selection is done over several
/// runs.
const PER_RUN: usize = 6;

/// Identifies a run over `photos` with these settings: a photo already done by a run with the
/// same token is skipped when the command is run again, so a large selection is done in turns.
fn run_token(photos: &[String], format: &Value, inset: f64) -> String {
    // FNV-1a: small and stable
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for p in photos {
        eat(p.as_bytes());
        eat(b",");
    }
    eat(format.to_string().as_bytes());
    eat(&inset.to_bits().to_le_bytes());
    format!("{h:016x}")
}

/// The key under which a photo keeps the token of the run that last cropped it.
const DONE_KEY: &str = "autocropRun";

fn autocrop(req: &Request) -> Result<Value, String> {
    let photos = req.selection();
    if photos.is_empty() {
        return Ok(json!({"message": "Select the photos to crop first."}));
    }
    let inset = req.arg("inset").as_f64().unwrap_or(0.4).clamp(0.0, 5.0) / 100.0;
    let opts = detect::Options {
        inset,
        format: format_arg(req.arg("format")),
        ..detect::Options::default()
    };
    // the photos this selection still needs; all of them again once every one was done
    let token = run_token(&photos, req.arg("format"), inset);
    let done = |p: &String| host::ns_get(p).is_ok_and(|v| v[DONE_KEY] == token.as_str());
    let mut todo: Vec<&String> = photos.iter().filter(|p| !done(p)).collect();
    if todo.is_empty() {
        todo = photos.iter().collect();
    }
    let left = todo.len().saturating_sub(PER_RUN);
    todo.truncate(PER_RUN);
    let (mut cropped, mut partial, mut none, mut failed) = (0usize, 0usize, Vec::new(), Vec::new());
    for &photo in &todo {
        let outcome = crop_one(photo, &opts);
        if let Outcome::Failed(e) = &outcome
            && let Some(missing) = missing_permission(e)
        {
            // every photo would fail the same way: say what to allow, once
            host::log(&format!("photo {photo}: {e}"));
            return Ok(json!({"message": format!(
                "ChromaGate needs the permission “{missing}”. Allow it in Plug-in Manager ▸ ChromaGate ▸ Permissions, then run it again."
            )}));
        }
        match outcome {
            Outcome::Cropped(d) => {
                cropped += 1;
                if d.sides() < 4 {
                    partial += 1;
                }
                let r = d.rect;
                let format = d.format.map_or("no standard format", |f| FORMATS[f].0);
                host::log(&format!(
                    "photo {photo}: crop [{:.4}, {:.4}, {:.4}, {:.4}], straightened {:+.2}°, edges found {}/4, {format}",
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    d.angle,
                    d.sides()
                ));
            }
            Outcome::NoFrame => {
                host::log(&format!(
                    "photo {photo}: no clear frame edge, left as it was"
                ));
                none.push(photo.clone());
            }
            Outcome::Failed(e) => {
                host::log(&format!("photo {photo}: {e}"));
                failed.push(photo.clone());
            }
        }
        if left > 0 || photos.len() > PER_RUN {
            // remembered only when the selection takes more than one run
            let _ = host::ns_set(photo, DONE_KEY, json!(token));
        }
    }
    let mut message = summary(todo.len(), cropped, partial, none.len(), failed.len());
    if left > 0 {
        let more = if left == 1 {
            "1 photo".to_string()
        } else {
            format!("{left} photos")
        };
        message.push_str(&format!(
            " {more} left: run Auto-Crop Film Frames again with the same selection to continue."
        ));
    }
    Ok(json!({
        "message": message,
        "cropped": cropped,
        "noFrame": none,
        "failed": failed,
        "left": left,
    }))
}

/// Most scans one run of Split splits: each takes a detection (as Auto-Crop's photos do), the
/// split, and a crop and a virtual copy per extra exposure.
const SPLIT_PER_RUN: usize = 4;

/// On a scan Split has split: the ids of its copies, one per extra exposure, in order.
const COPIES_KEY: &str = "splitCopies";
/// On a copy Split made: the scan it was made from (such a copy is never split itself).
const COPY_OF_KEY: &str = "splitCopyOf";
/// The key under which a scan keeps the token of the Split run that last did it.
const SPLIT_DONE_KEY: &str = "splitRun";

/// The dialog's "Exposures in each scan" choice.
fn count_arg(v: &Value) -> usize {
    v.as_str()
        .and_then(|s| s.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .filter(|n| (2..=4).contains(n))
        .unwrap_or(2)
}

/// The dialog's "Gate shape" choice.
fn gate_arg(v: &Value) -> Format {
    match v.as_str() {
        Some("Auto") => Format::Auto,
        Some("Any shape") => Format::Any,
        _ => Format::Only(0),
    }
}

/// Which exposure the original keeps (the dialog's "The original keeps"), of `n`, and the order
/// the copies take the others in: along the film, so copy 1 is the exposure next to the kept one.
fn exposure_order(v: &Value, n: usize) -> Vec<usize> {
    if v.as_str().is_some_and(|s| s.starts_with("The last")) {
        (0..n).rev().collect()
    } else {
        (0..n).collect()
    }
}

/// What happened to one scan.
enum SplitOutcome {
    /// Split into this many exposures; how many dividers were seen (the rest split evenly); whether
    /// the gate was found on every side.
    Split {
        exposures: usize,
        seen: usize,
        whole_gate: bool,
    },
    /// A copy Split made, which isn't split again.
    IsCopy,
    NoFrame,
    Failed(String),
}

/// Copy `k` (1-based) of `photo`, cropped to `rect` turned by `angle`: the one an earlier run made
/// (`known`, that run's list) when it's still in the library, else a new one. → its id.
fn crop_copy(
    photo: &str,
    k: usize,
    known: &[String],
    rect: [f64; 4],
    angle: f64,
) -> Result<String, String> {
    if let Some(id) = known
        .get(k - 1)
        .filter(|id| host::ns_get(id).is_ok_and(|v| v[COPY_OF_KEY] == photo))
    {
        match host::set_crop(id, rect, Some(angle)) {
            Ok(_) => return Ok(id.clone()),
            // removed from the library since: make another
            Err(e) if e.contains("no photo") => {}
            Err(e) => return Err(e),
        }
    }
    let id = new_copy(photo, k)?;
    host::set_crop(&id, rect, Some(angle))?;
    Ok(id)
}

/// A new virtual copy of `photo` for its exposure `k + 1`, marked as Split's.
fn new_copy(photo: &str, k: usize) -> Result<String, String> {
    let v = host::call(
        "photo.virtualCopy",
        json!({"photo": photo, "name": format!("Exposure {}", k + 1)}),
    )?;
    let id = v["photo"]
        .as_str()
        .ok_or("photo.virtualCopy: no `photo` in the reply")?
        .to_string();
    host::ns_set(&id, COPY_OF_KEY, json!(photo))?;
    Ok(id)
}

fn split_one(
    photo: &str,
    n: usize,
    keep: &Value,
    opts: &detect::Options,
    inset: f64,
) -> SplitOutcome {
    if host::ns_get(photo).is_ok_and(|v| v[COPY_OF_KEY].is_string()) {
        return SplitOutcome::IsCopy;
    }
    let px = match host::photo_pixels(photo, LOOK_EDGE) {
        Ok(px) => px,
        Err(e) => return SplitOutcome::Failed(e),
    };
    // the gate, flush (no trim: each exposure gets its own); the whole scan, level, when no gate shows
    let gate_opts = detect::Options {
        inset: 0.0,
        ..*opts
    };
    let d = detect::detect(&px.rgb, px.width, px.height, &gate_opts);
    let (gate, angle, whole_gate) = d.as_ref().map_or(([0.0, 0.0, 1.0, 1.0], 0.0, false), |d| {
        (d.rect, d.angle, d.sides() == 4)
    });
    let Some(s) = split::split(&px.rgb, px.width, px.height, gate, angle, n, inset) else {
        return SplitOutcome::NoFrame;
    };
    let order = exposure_order(keep, n);
    let known: Vec<String> = host::ns_get(photo)
        .ok()
        .and_then(|v| v[COPIES_KEY].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if let Err(e) = host::set_crop(photo, s.frames[order[0]], Some(angle)) {
        return SplitOutcome::Failed(e);
    }
    let mut copies = Vec::with_capacity(n - 1);
    for (k, &frame) in order.iter().enumerate().skip(1) {
        match crop_copy(photo, k, &known, s.frames[frame], angle) {
            Ok(id) => copies.push(id),
            Err(e) => {
                // keep what was made, so the next run reuses it
                let _ = host::ns_set(photo, COPIES_KEY, json!(copies));
                return SplitOutcome::Failed(e);
            }
        }
    }
    let _ = host::ns_set(photo, COPIES_KEY, json!(copies));
    let seen = s.found.iter().filter(|f| **f).count();
    host::log(&format!(
        "photo {photo}: {n} exposures, straightened {angle:+.2}°, dividers seen {seen}/{}, gate edges {}/4; original keeps exposure {}, copies {}",
        n - 1,
        d.as_ref().map_or(0, detect::Detection::sides),
        order[0] + 1,
        copies.join(", ")
    ));
    SplitOutcome::Split {
        exposures: n,
        seen,
        whole_gate,
    }
}

fn split_scans(req: &Request) -> Result<Value, String> {
    let photos = req.selection();
    if photos.is_empty() {
        return Ok(json!({"message": "Select the scans to split first."}));
    }
    let n = count_arg(req.arg("count"));
    let inset = req.arg("inset").as_f64().unwrap_or(1.0).clamp(0.0, 5.0) / 100.0;
    let opts = detect::Options {
        format: gate_arg(req.arg("gate")),
        ..detect::Options::default()
    };
    let settings = json!([req.arg("count"), req.arg("keep"), req.arg("gate")]);
    let token = run_token(&photos, &settings, inset);
    let done = |p: &String| host::ns_get(p).is_ok_and(|v| v[SPLIT_DONE_KEY] == token.as_str());
    let mut todo: Vec<&String> = photos.iter().filter(|p| !done(p)).collect();
    if todo.is_empty() {
        todo = photos.iter().collect();
    }
    let left = todo.len().saturating_sub(SPLIT_PER_RUN);
    todo.truncate(SPLIT_PER_RUN);
    let (mut split, mut copies, mut even, mut partial, mut skipped) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut none, mut failed) = (Vec::new(), Vec::new());
    for &photo in &todo {
        let outcome = split_one(photo, n, req.arg("keep"), &opts, inset);
        if let SplitOutcome::Failed(e) = &outcome
            && let Some(missing) = missing_permission(e)
        {
            host::log(&format!("photo {photo}: {e}"));
            return Ok(json!({"message": format!(
                "ChromaGate needs the permission “{missing}”. Allow it in Plug-in Manager ▸ ChromaGate ▸ Permissions, then run it again."
            )}));
        }
        match outcome {
            SplitOutcome::Split {
                exposures,
                seen,
                whole_gate,
            } => {
                split += 1;
                copies += exposures - 1;
                if seen < exposures - 1 {
                    even += 1;
                }
                if !whole_gate {
                    partial += 1;
                }
            }
            SplitOutcome::IsCopy => skipped += 1,
            SplitOutcome::NoFrame => {
                host::log(&format!(
                    "photo {photo}: too small to split, left as it was"
                ));
                none.push(photo.clone());
            }
            SplitOutcome::Failed(e) => {
                host::log(&format!("photo {photo}: {e}"));
                failed.push(photo.clone());
            }
        }
        if left > 0 || photos.len() > SPLIT_PER_RUN {
            let _ = host::ns_set(photo, SPLIT_DONE_KEY, json!(token));
        }
    }
    let mut message = split_summary(
        todo.len() - skipped,
        split,
        copies,
        even,
        partial,
        none.len(),
        failed.len(),
        skipped,
    );
    if left > 0 {
        let more = if left == 1 {
            "1 scan".to_string()
        } else {
            format!("{left} scans")
        };
        message.push_str(&format!(
            " {more} left: run Split Half-Frame Scans again with the same selection to continue."
        ));
    }
    Ok(json!({
        "message": message,
        "split": split,
        "copies": copies,
        "failed": failed,
        "left": left,
    }))
}

/// The notice shown when Split ends.
#[allow(clippy::too_many_arguments)]
fn split_summary(
    total: usize,
    split: usize,
    copies: usize,
    even: usize,
    partial: usize,
    none: usize,
    failed: usize,
    skipped: usize,
) -> String {
    let n = |k: usize, one: &str, many: &str| {
        if k == 1 {
            format!("1 {one}")
        } else {
            format!("{k} {many}")
        }
    };
    if total == 0 {
        return format!(
            "Nothing to split: the selection is {} Split made earlier.",
            n(skipped, "copy", "copies")
        );
    }
    let mut parts = vec![if split == total {
        format!("Split {}", n(total, "scan", "scans"))
    } else {
        format!("Split {split} of {}", n(total, "scan", "scans"))
    }];
    parts[0].push_str(&format!(
        " ({} cropped to the next exposure).",
        n(copies, "virtual copy", "virtual copies")
    ));
    if even > 0 {
        parts.push(format!(
            "No strip between the exposures in {}: split evenly, check those.",
            n(even, "scan", "scans")
        ));
    }
    if partial > 0 {
        parts.push(format!(
            "{} had a clear gate on only some sides; check those.",
            n(partial, "scan", "scans")
        ));
    }
    if none > 0 {
        parts.push(format!(
            "{} too small to split (left as it was).",
            n(none, "scan was", "scans were")
        ));
    }
    if failed > 0 {
        parts.push(format!(
            "{} couldn't be split (see the plug-in's log).",
            n(failed, "scan", "scans")
        ));
    }
    if skipped > 0 {
        parts.push(format!(
            "Skipped {} Split made earlier.",
            n(skipped, "copy", "copies")
        ));
    }
    parts.join(" ")
}

/// The permission a host error says this plug-in lacks, as the Plug-in Manager names it.
fn missing_permission(error: &str) -> Option<&'static str> {
    if !error.contains("not granted") {
        return None;
    }
    if error.contains("`pixels`") {
        Some("Look at downsized copies of photos")
    } else if error.contains("`crop`") {
        Some("Change the crop of photos")
    } else if error.contains("`virtualCopy`") {
        Some("Make virtual copies of photos")
    } else {
        None
    }
}

/// The notice shown when the command ends.
fn summary(total: usize, cropped: usize, partial: usize, none: usize, failed: usize) -> String {
    let photos = |n: usize| {
        if n == 1 {
            "1 photo".to_string()
        } else {
            format!("{n} photos")
        }
    };
    let mut parts = vec![if cropped == total {
        format!("Cropped {}.", photos(total))
    } else {
        format!("Cropped {cropped} of {}.", photos(total))
    }];
    if partial > 0 {
        parts.push(format!(
            "{} had a clear edge on only some sides; check those.",
            photos(partial)
        ));
    }
    if none > 0 {
        parts.push(format!(
            "No clear frame edge in {} (left as it was).",
            photos(none)
        ));
    }
    if failed > 0 {
        parts.push(format!(
            "{} couldn't be cropped (see the plug-in's log).",
            photos(failed)
        ));
    }
    parts.join(" ")
}

fn handle(req: &Request) -> Result<Value, String> {
    match (req.hook(), req.command()) {
        ("command", "autocrop") => autocrop(req),
        ("command", "split") => split_scans(req),
        (hook, cmd) => Err(format!("unsupported request {hook} {cmd}")),
    }
}

export_plugin!(&manifest(), handle);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_is_json_and_asks_only_for_pixels_crop_and_copies() {
        let m: Value = lcc_plugin_sdk::serde_json::from_str(&manifest()).unwrap();
        assert_eq!(m["id"], "io.github.nwh212.chromagate");
        assert_eq!(m["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            m["permissions"],
            json!({"pixels": true, "crop": true, "virtualCopy": true})
        );
        assert_eq!(m["requires"], json!(["pixels", "crop", "virtualCopy"]));
    }

    #[test]
    fn split_s_dialog_values_read_as_meant() {
        let m: Value = lcc_plugin_sdk::serde_json::from_str(&manifest()).unwrap();
        let dialog = m["commands"][1]["dialog"].as_array().unwrap();
        assert_eq!(m["commands"][1]["id"], "split");
        for o in dialog[0]["options"].as_array().unwrap() {
            assert!((2..=4).contains(&count_arg(o)), "{o}");
        }
        assert_eq!(count_arg(&json!("2 (half-frame)")), 2);
        assert_eq!(count_arg(&json!("4")), 4);
        assert_eq!(count_arg(&json!("nonsense")), 2);
        let keep = dialog[1]["options"].as_array().unwrap();
        assert_eq!(exposure_order(&keep[0], 3), vec![0, 1, 2]);
        assert_eq!(exposure_order(&keep[1], 3), vec![2, 1, 0]);
        let gates = dialog[2]["options"].as_array().unwrap();
        assert_eq!(gate_arg(&gates[0]), Format::Only(0));
        assert_eq!(FORMATS[0].0, "35mm / 6×9 (3:2)", "the 35 mm gate is 3:2");
        assert_eq!(gate_arg(&gates[1]), Format::Auto);
        assert_eq!(gate_arg(&gates[2]), Format::Any);
    }

    #[test]
    fn split_s_summary_says_what_needs_a_look() {
        assert_eq!(
            split_summary(2, 2, 2, 0, 0, 0, 0, 0),
            "Split 2 scans (2 virtual copies cropped to the next exposure)."
        );
        assert_eq!(
            split_summary(3, 2, 2, 1, 1, 0, 1, 1),
            "Split 2 of 3 scans (2 virtual copies cropped to the next exposure). No strip between the exposures in 1 scan: split evenly, check those. 1 scan had a clear gate on only some sides; check those. 1 scan couldn't be split (see the plug-in's log). Skipped 1 copy Split made earlier."
        );
        assert_eq!(
            split_summary(0, 0, 0, 0, 0, 0, 0, 2),
            "Nothing to split: the selection is 2 copies Split made earlier."
        );
    }

    #[test]
    fn the_pictures_are_svg_documents_within_the_host_s_limit() {
        let m: Value = lcc_plugin_sdk::serde_json::from_str(&manifest()).unwrap();
        for svg in [&m["icon"], &m["header"]["dark"], &m["header"]["light"]] {
            let svg = svg.as_str().unwrap();
            assert!(
                svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"),
                "{svg:.40}"
            );
            assert!(svg.len() <= 24 * 1024);
            // drawn as untrusted documents: nothing linked or embedded
            assert!(!svg.contains("<image") && !svg.contains("href"));
        }
    }

    #[test]
    fn every_format_in_the_dialog_is_one_the_detector_knows() {
        let m: Value = lcc_plugin_sdk::serde_json::from_str(&manifest()).unwrap();
        let options = m["commands"][0]["dialog"][0]["options"].as_array().unwrap();
        assert_eq!(options.len(), FORMATS.len() + 2);
        for (i, (name, _, _)) in FORMATS.iter().enumerate() {
            assert_eq!(options[i + 1], *name);
            assert_eq!(format_arg(&json!(name)), Format::Only(i));
        }
        assert_eq!(format_arg(&json!("Auto")), Format::Auto);
        assert_eq!(format_arg(&json!("Any shape")), Format::Any);
    }

    #[test]
    fn a_run_s_token_follows_the_selection_and_the_settings() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let t = run_token(&p(&["1", "2"]), &json!("Auto"), 0.004);
        assert_eq!(t, run_token(&p(&["1", "2"]), &json!("Auto"), 0.004));
        assert_ne!(t, run_token(&p(&["1", "3"]), &json!("Auto"), 0.004));
        assert_ne!(t, run_token(&p(&["12"]), &json!("Auto"), 0.004));
        assert_ne!(t, run_token(&p(&["1", "2"]), &json!("Any shape"), 0.004));
        assert_ne!(t, run_token(&p(&["1", "2"]), &json!("Auto"), 0.005));
    }

    #[test]
    fn a_missing_permission_is_named_as_the_plug_in_manager_shows_it() {
        assert_eq!(
            missing_permission("permission `pixels` not granted to this plug-in"),
            Some("Look at downsized copies of photos")
        );
        assert_eq!(
            missing_permission("permission `crop` not granted to this plug-in"),
            Some("Change the crop of photos")
        );
        assert_eq!(
            missing_permission("permission `virtualCopy` not granted to this plug-in"),
            Some("Make virtual copies of photos")
        );
        assert_eq!(missing_permission("photo.pixels: no photo 9"), None);
    }

    #[test]
    fn the_summary_says_what_needs_a_look() {
        assert_eq!(summary(3, 3, 0, 0, 0), "Cropped 3 photos.");
        assert_eq!(summary(1, 1, 0, 0, 0), "Cropped 1 photo.");
        assert_eq!(
            summary(5, 3, 1, 1, 1),
            "Cropped 3 of 5 photos. 1 photo had a clear edge on only some sides; check those. No clear frame edge in 1 photo (left as it was). 1 photo couldn't be cropped (see the plug-in's log)."
        );
    }
}
