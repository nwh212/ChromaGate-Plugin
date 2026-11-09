//! Finds the picture area of a film frame, whatever state it is in: a raw negative with its
//! sprocket holes, a converted negative on a light table, a slide in its mount, a frame scanned on
//! white or black, a thin negative whose gate barely shows.
//!
//! What all of them share: the picture is bounded by one long, straight edge that runs across
//! (nearly) the whole frame. For each side we score every row (or column) in the outer band of the
//! image by how *coherent* the edge across it is, and keep the strong ones as candidates.
//!
//! A line's score has two parts:
//! - **strength**: the mean signed gradient across the line (over its middle 80%, so corners and
//!   neighbouring frames don't count), in the colour channel where it is largest;
//! - **coverage**: the fraction of the line where the gradient is significant and has that sign. A
//!   picture edge against a dark rebate fades out where the picture is dark too, so a line where
//!   almost nothing pushes the *other* way counts as more coherent than its raw coverage.
//!
//! A row through a picture (a horizon, a bench) has edges of both signs and low coverage; a row
//! through sprocket holes alternates sign and cancels out; the frame edge is one sign end to end.
//!
//! A scene can still hold a long straight line (a wall, the base of a building), so the four
//! edges chosen are the ones that make a frame of a real film format (3:2, 6×4.5 / 4:3, 6×6,
//! 6×7 / 4×5): a format's shape first, then the most real edges, then the more common format,
//! then the innermost box, so the rebate, sprocket holes and mount are cut away. When nothing
//! makes such a frame, the most real edges and the innermost box win.
//!
//! Each colour channel is first stretched to the image's own range, so a thin negative's faint
//! gate (a picture barely darker or yellower than its rebate) still gives an edge.

/// Film formats as ranges of long side ÷ short side, the most common first.
pub const FORMATS: &[(&str, f64, f64)] = &[
    ("35mm / 6×9 (3:2)", 1.46, 1.54),
    ("6×4.5 / half-frame (4:3)", 1.30, 1.38),
    ("6×6 (1:1)", 0.97, 1.03),
    ("6×7 / 4×5", 1.20, 1.29),
];

/// Which frame shapes [`detect`] looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Any of [`FORMATS`]; the innermost edges when none fits.
    Auto,
    /// Only [`FORMATS`]`[i]`; the innermost edges when it doesn't fit.
    Only(usize),
    /// No shape: the innermost edges.
    Any,
}

/// What [`detect`] may be told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Fraction of each dimension searched from each side.
    pub band: f64,
    /// Moved inwards from each edge found, as a fraction of the dimension: hides the soft line
    /// the frame edge leaves.
    pub inset: f64,
    pub format: Format,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            band: 0.30,
            inset: 0.004,
            format: Format::Auto,
        }
    }
}

/// Where the picture is: `rect` `[x0, y0, x1, y1]`, normalized; `found` tells, per side (left,
/// top, right, bottom), whether an edge was found there (a side without one stays at the image
/// edge); `format`, the film format the box matches ([`FORMATS`] index), if any; `angle`, how far
/// to straighten it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    pub rect: [f64; 4],
    pub found: [bool; 4],
    pub format: Option<usize>,
    /// The crop's straighten angle, degrees, clockwise positive (0: the frame lies straight);
    /// `rect` is in the frame turned by it, as Chromaforge's crop is.
    pub angle: f64,
}

impl Detection {
    /// Sides with an edge found.
    pub fn sides(&self) -> usize {
        self.found.iter().filter(|f| **f).count()
    }
}

/// Part of a line used for scoring: its middle, away from corners and neighbouring frames.
const SPAN: (f64, f64) = (0.10, 0.90);
/// Coverage a line needs to be an edge at all.
const MIN_COVERAGE: f64 = 0.65;
/// Below this share of a line pushing the other way, coverage gets [`CLEAN_BONUS`].
const CLEAN_CONTRA: f64 = 0.08;
const CLEAN_BONUS: f64 = 0.2;
/// A candidate edge is at least this strong, and at least [`REL_STRENGTH`] of the band's strongest.
const MIN_STRENGTH: f64 = 0.03;
const REL_STRENGTH: f64 = 0.08;
/// Most candidates kept per side (the strongest).
const MAX_CANDIDATES: usize = 12;
/// Share of each channel's darkest and brightest samples clipped by the stretch.
const STRETCH_CLIP: f64 = 0.005;
/// The picture is at least this fraction of each dimension; smaller means something went wrong.
const MIN_SIDE: f64 = 0.25;

/// Three planes (R, G, B) of `w × h` samples in 0..1, rows top to bottom.
///
/// Everything here is written for the plug-in sandbox, an interpreter where every instruction
/// counts: one pass to fill the planes, whole rows as slices, lookup tables, no blurred or
/// transposed copies.
struct Planes {
    w: usize,
    h: usize,
    c: [Vec<f32>; 3],
}

impl Planes {
    /// From interleaved 8-bit RGB, each channel stretched so its darkest and brightest
    /// [`STRETCH_CLIP`] land at 0 and 1.
    fn from_rgb8(rgb: &[u8], w: usize, h: usize) -> Planes {
        let rgb = &rgb[..w * h * 3];
        // the range from every 4th pixel: as good, for a quarter of the work
        let mut hist = [[0u32; 256]; 3];
        let mut seen = 0u32;
        for px in rgb.as_chunks::<12>().0 {
            hist[0][usize::from(px[0])] += 1;
            hist[1][usize::from(px[1])] += 1;
            hist[2][usize::from(px[2])] += 1;
            seen += 1;
        }
        let clip = (f64::from(seen) * STRETCH_CLIP) as u32;
        let lut: [[f32; 256]; 3] = std::array::from_fn(|k| {
            let (mut lo, mut acc) = (0usize, 0u32);
            while lo < 255 && acc + hist[k][lo] <= clip {
                acc += hist[k][lo];
                lo += 1;
            }
            let (mut hi, mut acc) = (255usize, 0u32);
            while hi > 0 && acc + hist[k][hi] <= clip {
                acc += hist[k][hi];
                hi -= 1;
            }
            // a flat channel isn't blown up into noise
            let span = (hi as f32 - lo as f32).max(3.0);
            std::array::from_fn(|v| ((v as f32 - lo as f32) / span).clamp(0.0, 1.0))
        });
        let mut c = [
            Vec::with_capacity(w * h),
            Vec::with_capacity(w * h),
            Vec::with_capacity(w * h),
        ];
        for px in rgb.as_chunks::<3>().0 {
            c[0].push(lut[0][usize::from(px[0])]);
            c[1].push(lut[1][usize::from(px[1])]);
            c[2].push(lut[2][usize::from(px[2])]);
        }
        Planes { w, h, c }
    }
}

/// Per line, its edge strength and coverage (see the module docs).
struct Lines {
    strength: Vec<f64>,
    coverage: Vec<f64>,
    /// Whether the edge across the line goes from darker to brighter (in its channel).
    rising: Vec<bool>,
}

/// The lines of one dimension that are scored: those in the bands `band` deep at either end (no
/// other line is ever picked), each with the 2 lines on either side it reads.
fn band_lines(n: usize, band: f64) -> impl Iterator<Item = usize> {
    let b = ((n as f64 * band) as usize + 1).min(n);
    (1..n.saturating_sub(1)).filter(move |&i| i < b || i >= n - b)
}

/// Strength and coverage of a line from its gradients `g` (one per sample of its span) in the
/// channel where the mean `m` is largest.
fn line_score(g: &[f32], m: f32) -> (f64, f64) {
    let thr = (m.abs() * 0.25).max(0.01);
    let (mut pos, mut neg) = (0usize, 0usize);
    for &v in g {
        pos += usize::from(v > thr);
        neg += usize::from(v < -thr);
    }
    let (same, contra) = if m > 0.0 { (pos, neg) } else { (neg, pos) };
    let len = g.len().max(1) as f64;
    let (same, contra) = (same as f64 / len, contra as f64 / len);
    (
        f64::from(m.abs()),
        if contra <= CLEAN_CONTRA {
            (same + CLEAN_BONUS).min(1.0)
        } else {
            same
        },
    )
}

/// Scores the horizontal lines of `p`: the gradient across line `i` is the mean of rows `i+1` and
/// `i+2` minus that of rows `i-1` and `i-2` (smoothing across the edge, where grain would
/// otherwise break it up), over the middle [`SPAN`] of the row.
fn score_rows(p: &Planes, band: f64) -> Lines {
    let (n, w) = (p.h, p.w);
    let (mut strength, mut coverage, mut rising) = (vec![0.0; n], vec![0.0; n], vec![false; n]);
    if n < 5 || w < 5 {
        return Lines {
            strength,
            coverage,
            rising,
        };
    }
    let (lo, hi) = ((w as f64 * SPAN.0) as usize, (w as f64 * SPAN.1) as usize);
    let inv = 1.0 / (hi - lo).max(1) as f32;
    let row = |y: usize| -> std::ops::Range<usize> { y * w + lo..y * w + hi };
    let mut g: [Vec<f32>; 3] = std::array::from_fn(|_| vec![0.0f32; hi - lo]);
    for i in band_lines(n, band) {
        let (a2, a1, b1, b2) = (i.saturating_sub(2), i - 1, i + 1, (i + 2).min(n - 1));
        let mut mean = [0.0f32; 3];
        for k in 0..3 {
            let c = &p.c[k];
            let (r_a2, r_a1, r_b1, r_b2) = (&c[row(a2)], &c[row(a1)], &c[row(b1)], &c[row(b2)]);
            let mut sum = 0.0f32;
            for ((((d, &x2), &x1), &y1), &y2) in
                g[k].iter_mut().zip(r_a2).zip(r_a1).zip(r_b1).zip(r_b2)
            {
                let v = (y1 + y2 - x1 - x2) * (1.0 / 3.0);
                *d = v;
                sum += v;
            }
            mean[k] = sum * inv;
        }
        let k = (0..3).fold(0, |best, k| {
            if mean[k].abs() > mean[best].abs() {
                k
            } else {
                best
            }
        });
        (strength[i], coverage[i]) = line_score(&g[k], mean[k]);
        rising[i] = mean[k] > 0.0;
    }
    Lines {
        strength,
        coverage,
        rising,
    }
}

/// Columns `cols` of `p` turned into rows: a `h × cols.len()` strip, its row `i` being column
/// `cols.start + i`.
fn columns_as_rows(p: &Planes, cols: std::ops::Range<usize>) -> Planes {
    let (w, h, n) = (p.w, p.h, cols.len());
    let c = std::array::from_fn(|k| {
        let mut out = vec![0.0f32; n * h];
        for (y, row) in p.c[k].chunks_exact(w).enumerate() {
            for (i, &v) in row[cols.clone()].iter().enumerate() {
                out[i * h + y] = v;
            }
        }
        out
    });
    Planes { w: h, h: n, c }
}

/// Scores the vertical lines of `p` as [`score_rows`] does the horizontal ones: the bands at the
/// left and right (with the lines the outermost of them read) turned into rows and scored whole.
fn score_cols(p: &Planes, band: f64) -> Lines {
    let w = p.w;
    let (mut strength, mut coverage, mut rising) = (vec![0.0; w], vec![0.0; w], vec![false; w]);
    // a band's lines, and the 2 beyond them its innermost lines read
    let b = ((w as f64 * band) as usize + 4).min(w);
    for start in [0, w - b] {
        let strip = score_rows(&columns_as_rows(p, start..start + b), 1.0);
        // a line read past the strip's inner end is outside the band: it's never picked
        strength[start..start + b].copy_from_slice(&strip.strength);
        coverage[start..start + b].copy_from_slice(&strip.coverage);
        rising[start..start + b].copy_from_slice(&strip.rising);
    }
    Lines {
        strength,
        coverage,
        rising,
    }
}

/// The candidate edges in the band at the start (`from_end` false) or end of `l`: their lines,
/// at most [`MAX_CANDIDATES`], the strongest.
fn candidates(l: &Lines, band: f64, from_end: bool) -> Vec<usize> {
    let n = l.strength.len();
    let b = ((n as f64 * band) as usize).min(n);
    let idx: Vec<usize> = if from_end {
        (n - b..n).collect()
    } else {
        (0..b).collect()
    };
    let ok = |i: usize| l.coverage[i] >= MIN_COVERAGE;
    let smax = idx
        .iter()
        .copied()
        .filter(|&i| ok(i))
        .map(|i| l.strength[i])
        .fold(0.0, f64::max);
    if smax <= 0.0 {
        return Vec::new();
    }
    let bar = MIN_STRENGTH.max(REL_STRENGTH * smax);
    let mut out: Vec<usize> = idx
        .iter()
        .enumerate()
        .filter(|&(k, &i)| {
            // a peak among the lines around it that go the same way: an edge's blur spreads over
            // a few lines, but a slide's mask or a thin rebate can put the next edge, the other
            // way, only 3 lines away
            let window = &idx[k.saturating_sub(2)..(k + 3).min(idx.len())];
            ok(i)
                && l.strength[i] >= bar
                && window
                    .iter()
                    .all(|&j| l.rising[j] != l.rising[i] || l.strength[i] >= l.strength[j])
        })
        .map(|(_, &i)| i)
        .collect();
    out.sort_by(|&a, &b| l.strength[b].total_cmp(&l.strength[a]));
    out.truncate(MAX_CANDIDATES);
    out
}

/// The format `format` allows that a `w × h` box matches (an index into [`FORMATS`]).
fn format_of(w: f64, h: f64, format: Format) -> Option<usize> {
    let r = if w >= h { w / h } else { h / w };
    let fits = |i: usize| {
        FORMATS
            .get(i)
            .is_some_and(|&(_, lo, hi)| (lo..=hi).contains(&r))
    };
    match format {
        Format::Auto => (0..FORMATS.len()).find(|&i| fits(i)),
        Format::Only(i) => fits(i).then_some(i),
        Format::Any => None,
    }
}

/// Without a frame shape to go by, a side steps inwards from its outermost edge only across strips
/// narrower than this fraction of the dimension: a rebate between a holder and the gate, not a
/// quarter of the picture.
const MAX_STRIP: f64 = 0.08;

/// The best box of four real edges that has a film format's shape: the more common format, then
/// the smaller box. `sides` are the candidates of left, top, right, bottom.
fn format_box(
    sides: &[Vec<usize>; 4],
    w: f64,
    h: f64,
    format: Format,
) -> Option<([usize; 4], usize)> {
    let mut best: Option<((isize, f64), [usize; 4], usize)> = None;
    for &l in &sides[0] {
        for &t in &sides[1] {
            for &r in &sides[2] {
                for &b in &sides[3] {
                    let (bw, bh) = (r as f64 - l as f64, b as f64 - t as f64);
                    if bw < MIN_SIDE * w || bh < MIN_SIDE * h {
                        continue;
                    }
                    let Some(f) = format_of(bw, bh, format) else {
                        continue;
                    };
                    let rank = (-(f as isize), 1.0 - bw * bh / (w * h));
                    if best.as_ref().is_none_or(|(k, _, _)| rank > *k) {
                        best = Some((rank, [l, t, r, b], f));
                    }
                }
            }
        }
    }
    best.map(|(_, b, f)| (b, f))
}

/// One side's edge without a frame shape: the innermost candidate (`innermost`), or the outermost
/// one and inwards across narrow strips. `from_end`: the side at the end of the dimension `n`.
fn side_edge(cands: &[usize], n: usize, from_end: bool, innermost: bool) -> Option<usize> {
    // from the outside in
    let mut v = cands.to_vec();
    if from_end {
        v.sort_unstable_by(|a, b| b.cmp(a));
    } else {
        v.sort_unstable();
    }
    let (&first, rest) = v.split_first()?;
    if innermost {
        return v.last().copied();
    }
    let strip = (MAX_STRIP * n as f64).ceil() as usize;
    let mut at = first;
    for &c in rest {
        if c.abs_diff(at) > strip {
            break;
        }
        at = c;
    }
    Some(at)
}

/// Tilts smaller than this (degrees) count as square: about a pixel across a frame, and what a
/// scanner's mask, square to the sensor, measures.
const MIN_TILT: f64 = 0.15;
/// Tilts larger than this (degrees) are taken for a misreading.
const MAX_TILT: f64 = 5.0;
/// Sides whose tilts agree within this (degrees) confirm each other.
const TILT_AGREE: f64 = 0.25;
/// Segments a side is measured in, and how far (lines) each looks either side of the edge.
const SEGMENTS: usize = 16;
const REACH: usize = 6;

/// One edge's measured tilt: degrees clockwise positive, how many of the [`SEGMENTS`] pieces lie
/// on its line, and how far from it they lie (RMS, pixels).
#[derive(Clone, Copy, Debug)]
struct Tilt {
    degrees: f64,
    on_line: usize,
    rms: f64,
}

impl Tilt {
    /// Straight enough to trust on its own: nearly every piece on one line, within a fraction of
    /// a pixel.
    fn clean(&self) -> bool {
        self.on_line >= SEGMENTS - 2 && self.rms <= 0.35
    }
    fn tilted(&self) -> bool {
        self.degrees.abs() >= MIN_TILT
    }
}

/// The tilt of the edge found at line `line` of `p` (horizontal lines when `rows`): where the
/// edge lies in each of [`SEGMENTS`] pieces of its middle, then a straight line through them
/// (outliers dropped). `None` when too few pieces see it.
fn edge_tilt(p: &Planes, rows: bool, line: usize) -> Option<Tilt> {
    let (n, along) = if rows { (p.h, p.w) } else { (p.w, p.h) };
    // as far either side as the image allows, up to REACH
    let reach = REACH
        .min(line.saturating_sub(2))
        .min(n.saturating_sub(line + 3));
    if reach < 2 {
        return None;
    }
    let at = |c: &[f32], l: usize, i: usize| if rows { c[l * p.w + i] } else { c[i * p.w + l] };
    let grad = |c: &[f32], l: usize, i: usize| {
        (at(c, l + 1, i) + at(c, l + 2, i) - at(c, l - 1, i) - at(c, l - 2, i)) / 3.0
    };
    let (lo, hi) = (
        (along as f64 * SPAN.0) as usize,
        (along as f64 * SPAN.1) as usize,
    );
    // the channel and sign of the edge, over the whole middle
    let mean = |c: &[f32]| (lo..hi).map(|i| grad(c, line, i)).sum::<f32>();
    let (k, sign) = (0..3)
        .map(|k| (k, mean(&p.c[k])))
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))?;
    let (c, sign) = (&p.c[k], sign.signum());
    let seg = (hi - lo) / SEGMENTS;
    if seg < 2 {
        return None;
    }
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(SEGMENTS);
    for s in 0..SEGMENTS {
        let (a, z) = (lo + s * seg, lo + (s + 1) * seg);
        let profile: Vec<f32> = (line - reach..=line + reach)
            .map(|l| sign * (a..z).map(|i| grad(c, l, i)).sum::<f32>() / seg as f32)
            .collect();
        let (best, &peak) = profile
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))?;
        // a piece where the edge is lost (or at the reach's end) says nothing
        if peak < 0.02 || best == 0 || best == profile.len() - 1 {
            continue;
        }
        // the peak between samples
        let (l, m, r) = (profile[best - 1], peak, profile[best + 1]);
        let d = l - 2.0 * m + r;
        let sub = if d < 0.0 { 0.5 * (l - r) / d } else { 0.0 };
        pts.push((
            ((a + z) as f64) / 2.0,
            (line - reach + best) as f64 + f64::from(sub),
        ));
    }
    // a straight line through the pieces, then again without those far from it
    let fit = |pts: &[(f64, f64)]| -> Option<(f64, f64)> {
        let n = pts.len() as f64;
        let (sx, sy) = pts.iter().fold((0.0, 0.0), |(x, y), p| (x + p.0, y + p.1));
        let (mx, my) = (sx / n, sy / n);
        let (sxx, sxy) = pts.iter().fold((0.0, 0.0), |(a, b), p| {
            (a + (p.0 - mx).powi(2), b + (p.0 - mx) * (p.1 - my))
        });
        (sxx > 0.0).then(|| (sxy / sxx, my - sxy / sxx * mx))
    };
    if pts.len() < SEGMENTS / 2 {
        return None;
    }
    let (slope, icpt) = fit(&pts)?;
    let kept: Vec<(f64, f64)> = pts
        .iter()
        .copied()
        .filter(|p| (p.1 - (slope * p.0 + icpt)).abs() <= 1.0)
        .collect();
    if kept.len() < SEGMENTS / 2 {
        return None;
    }
    let (slope, icpt) = fit(&kept)?;
    let rms = (kept
        .iter()
        .map(|p| (p.1 - (slope * p.0 + icpt)).powi(2))
        .sum::<f64>()
        / kept.len() as f64)
        .sqrt();
    // a horizontal edge turned clockwise by φ rises as tan φ; a vertical one leans as −tan φ
    let tilt = if rows { slope.atan() } else { (-slope).atan() };
    Some(Tilt {
        degrees: tilt.to_degrees(),
        on_line: kept.len(),
        rms,
    })
}

/// The tilt of the frame (degrees, clockwise positive) from the tilts of its found edges.
///
/// An edge that lies square is most often the scanner's mask or holder, which is square to the
/// sensor while the film in it is not, so it doesn't speak against a tilt. The tilt is that of two
/// tilted edges that agree, else the mean of the clean ones ([`Tilt::clean`]) when they all lean
/// the same way; else 0. Small or implausible tilts are 0.
fn frame_tilt(p: &Planes, edges: &[Option<usize>; 4]) -> f64 {
    // each found edge's tilt, with the length it was measured over (a longer edge reads truer)
    let tilts: Vec<(Tilt, f64)> = edges
        .iter()
        .enumerate()
        .filter_map(|(k, e)| {
            e.and_then(|l| edge_tilt(p, k % 2 == 1, l))
                .map(|t| (t, if k % 2 == 1 { p.w } else { p.h } as f64))
        })
        .collect();
    let tilted: Vec<&(Tilt, f64)> = tilts
        .iter()
        .filter(|(t, _)| t.tilted() && t.degrees.abs() <= MAX_TILT)
        .collect();
    // two that agree
    for (i, a) in tilted.iter().enumerate() {
        let agreeing: Vec<f64> = tilted
            .iter()
            .skip(i)
            .filter(|b| (b.0.degrees - a.0.degrees).abs() <= TILT_AGREE)
            .map(|b| b.0.degrees)
            .collect();
        if agreeing.len() >= 2 {
            return agreeing.iter().sum::<f64>() / agreeing.len() as f64;
        }
    }
    // clean ones that lean the same way (a scan can make a frame slightly out of square): their
    // mean, by length; clean ones that lean opposite ways leave the frame as it is
    let clean: Vec<&&(Tilt, f64)> = tilted.iter().filter(|(t, _)| t.clean()).collect();
    if clean.is_empty()
        || !(clean.iter().all(|(t, _)| t.degrees > 0.0)
            || clean.iter().all(|(t, _)| t.degrees < 0.0))
    {
        return 0.0;
    }
    let length: f64 = clean.iter().map(|(_, l)| l).sum();
    clean.iter().map(|(t, l)| t.degrees * l).sum::<f64>() / length
}

impl Planes {
    /// The planes turned by `angle` degrees clockwise about their centre, as the crop's rotated
    /// frame turns the photo: a pixel's place here, normalized, is its place in that frame.
    /// Bilinear; what comes in from beyond the edges repeats them (so it makes no edge of its own).
    fn rotated(&self, angle: f64) -> Planes {
        let (w, h) = (self.w, self.h);
        let (s, c) = angle.to_radians().sin_cos();
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        // clamped to stop one short of the last sample, so its right / lower neighbour exists
        let (wm, hm) = ((w - 1) as f32 - 1e-3, (h - 1) as f32 - 1e-3);
        let mut out: [Vec<f32>; 3] = std::array::from_fn(|_| vec![0.0f32; w * h]);
        let (step_x, step_y) = (c as f32, -s as f32);
        for y in 0..h {
            let dy = y as f64 + 0.5 - cy;
            let dx = 0.5 - cx;
            // back into the photo (turned the other way), stepping along the row
            let (mut sx, mut sy) = (
                (c * dx + s * dy + cx - 0.5) as f32,
                (-s * dx + c * dy + cy - 0.5) as f32,
            );
            let row = y * w;
            for x in 0..w {
                let (px, py) = (sx.clamp(0.0, wm), sy.clamp(0.0, hm));
                sx += step_x;
                sy += step_y;
                let (x0, y0) = (px as usize, py as usize);
                let (fx, fy) = (px - x0 as f32, py - y0 as f32);
                let (i00, i10) = (y0 * w + x0, (y0 + 1) * w + x0);
                for (o, p) in out.iter_mut().zip(&self.c) {
                    let top = p[i00] + (p[i00 + 1] - p[i00]) * fx;
                    let bot = p[i10] + (p[i10 + 1] - p[i10]) * fx;
                    o[row + x] = top + (bot - top) * fy;
                }
            }
        }
        Planes { w, h, c: out }
    }
}

/// A weaker edge just inside a found one (or inside the image's border) is the gate when at least
/// this much of its line agrees: a soft, wavy gate edge, or a dark picture against a dark band,
/// leaves less than [`MIN_COVERAGE`].
const GATE_COVERAGE: f64 = 0.45;
/// The tone of the strip of lines `a..z` (rows when `rows`, else columns) of `p`, over `along` (a
/// range across the other dimension), when it is a band of film past the gate, a rebate or a mask: nearly
/// all one tone ([`BAND_SHARE`] of its samples within [`BAND_SPREAD`] of its median, in every
/// channel), and that tone near black or near white ([`BAND_EXTREME`]). Edge print or a speck of
/// dust in it doesn't change that; a strip of scene, busy or of a middle tone, isn't one.
fn band_tone(
    p: &Planes,
    rows: bool,
    a: usize,
    z: usize,
    along: std::ops::Range<usize>,
) -> Option<[f32; 3]> {
    if z <= a || along.is_empty() {
        return None;
    }
    let mut tone = [0.0f32; 3];
    for (k, c) in p.c.iter().enumerate() {
        let mut v: Vec<f32> = Vec::with_capacity((z - a) * along.len());
        for line in a..z {
            for i in along.clone() {
                v.push(if rows {
                    c[line * p.w + i]
                } else {
                    c[i * p.w + line]
                });
            }
        }
        let mid = v.len() / 2;
        let median = *v.select_nth_unstable_by(mid, f32::total_cmp).1;
        let near = v
            .iter()
            .filter(|x| (**x - median).abs() <= BAND_SPREAD)
            .count();
        if (near as f64) < BAND_SHARE * v.len() as f64 {
            return None;
        }
        tone[k] = median;
    }
    let luma = tone.iter().sum::<f32>() / 3.0;
    (!(BAND_EXTREME..=1.0 - BAND_EXTREME).contains(&luma)).then_some(tone)
}

/// The share of the strip of lines `a..z` of `p` over `along` that looks like a band of tone
/// `tone`: within [`BAND_SPREAD`] of it in every channel.
fn band_share(
    p: &Planes,
    rows: bool,
    a: usize,
    z: usize,
    along: std::ops::Range<usize>,
    tone: [f32; 3],
) -> f64 {
    let (mut like, mut all) = (0usize, 0usize);
    for line in a..z {
        for i in along.clone() {
            let at = if rows { line * p.w + i } else { i * p.w + line };
            all += 1;
            like += usize::from((0..3).all(|k| (p.c[k][at] - tone[k]).abs() <= BAND_SPREAD));
        }
    }
    if all == 0 {
        1.0
    } else {
        like as f64 / all as f64
    }
}

/// See [`band_tone`]. Inside a gate, less than [`PICTURE_BAND_SHARE`] of the picture looks like the
/// band (a night sky running on into more night sky is no band).
const BAND_SHARE: f64 = 0.75;
const BAND_SPREAD: f32 = 0.1;
const BAND_EXTREME: f32 = 0.2;
const PICTURE_BAND_SHARE: f64 = 0.6;

/// The weaker edges a side may step in to (see [`GATE_COVERAGE`]): peaks among the lines around
/// them that go the same way, in the band at the start (`from_end` false) or end of `l`.
fn gate_candidates(l: &Lines, band: f64, from_end: bool) -> Vec<usize> {
    let n = l.strength.len();
    let b = ((n as f64 * band) as usize).min(n);
    let idx: Vec<usize> = if from_end {
        (n - b..n).collect()
    } else {
        (0..b).collect()
    };
    idx.iter()
        .enumerate()
        .filter(|&(k, &i)| {
            let window = &idx[k.saturating_sub(2)..(k + 3).min(idx.len())];
            l.coverage[i] >= GATE_COVERAGE
                && l.strength[i] >= MIN_STRENGTH
                && window
                    .iter()
                    .all(|&j| l.rising[j] != l.rising[i] || l.strength[i] >= l.strength[j])
        })
        .map(|(_, &i)| i)
        .collect()
}

/// Steps each side of `edges` inwards past a band: from its edge (or from the image's border, when
/// it has none) to the nearest weaker edge inside, within [`MAX_STRIP`], where the strip between
/// them is a band ([`band_tone`]) and the picture inside it is unlike the band. That's the black band a gate's shadow
/// and the film past the gate leave around the picture, or a slide's mask, whose inner edge is
/// often too soft or too dark against the picture to be a candidate on its own.
// `k` is the side (left, top, right, bottom), not only an index into `edges`
#[allow(clippy::needless_range_loop)]
fn past_bands(p: &Planes, rows: &Lines, cols: &Lines, band: f64, edges: &mut [Option<usize>; 4]) {
    let (w, h) = (p.w, p.h);
    let span = |n: usize| (n as f64 * SPAN.0) as usize..(n as f64 * SPAN.1) as usize;
    for k in 0..4 {
        let (lines, n, from_end) = if k % 2 == 0 {
            (cols, w, k >= 2)
        } else {
            (rows, h, k >= 2)
        };
        let (rows_side, along) = if k % 2 == 0 {
            (false, span(h))
        } else {
            (true, span(w))
        };
        let strip_max = (MAX_STRIP * n as f64).ceil() as usize;
        let gates = gate_candidates(lines, band, from_end);
        let e = edges[k].unwrap_or(if from_end { n - 1 } else { 0 });
        let inward = |g: usize| {
            if from_end {
                g < e && e - g <= strip_max
            } else {
                g > e && g - e <= strip_max
            }
        };
        let mut inner: Vec<usize> = gates.iter().copied().filter(|&g| inward(g)).collect();
        inner.sort_by_key(|&g| g.abs_diff(e));
        // from the nearest inwards: the strip from the edge to the gate must be a band, and the
        // picture inside the gate unlike it; a gate with more band inside it (a faint line in
        // the band, or one band inside another) hands on to the next
        for g in inner {
            let (a, z) = (e.min(g) + 2, e.max(g).saturating_sub(2));
            if z <= a {
                // within the edge's own blur
                continue;
            }
            let Some(tone) = band_tone(p, rows_side, a, z, along.clone()) else {
                break;
            };
            // the picture just inside the gate, as wide as the band (at least a few lines)
            let width = (z - a).max(3);
            let (ia, iz) = if from_end {
                (g.saturating_sub(2 + width), g.saturating_sub(2))
            } else {
                (g + 2, (g + 2 + width).min(n))
            };
            if band_share(p, rows_side, ia, iz, along.clone(), tone) < PICTURE_BAND_SHARE {
                edges[k] = Some(past_wavy_gate(
                    p,
                    rows_side,
                    g,
                    from_end,
                    n,
                    along.clone(),
                    tone,
                ));
                break;
            }
        }
    }
}

/// A gate's edge can be wavy, the band reaching further in along part of the side: from the gate
/// at `g`, each line further in that still has more than [`WAVY_SHARE`] of band in it (within
/// [`WAVY_SPREAD`] of the band's `tone`, tight enough that a dark part of the picture isn't
/// taken for it) is cut away too, at most [`WAVY_MAX`] of the side `n` further.
fn past_wavy_gate(
    p: &Planes,
    rows: bool,
    g: usize,
    from_end: bool,
    n: usize,
    along: std::ops::Range<usize>,
    tone: [f32; 3],
) -> usize {
    let most = (WAVY_MAX * n as f64).ceil() as usize;
    let share = |line: usize| {
        let like = along
            .clone()
            .filter(|&i| {
                let at = if rows { line * p.w + i } else { i * p.w + line };
                (0..3).all(|k| (p.c[k][at] - tone[k]).abs() <= WAVY_SPREAD)
            })
            .count();
        like as f64 / along.len().max(1) as f64
    };
    let mut last = None;
    for step in 1..=most {
        let line = if from_end {
            g.checked_sub(step)
        } else {
            Some(g + step).filter(|&l| l < n)
        };
        let Some(line) = line else { break };
        if share(line) <= WAVY_SHARE {
            break;
        }
        last = Some(line);
    }
    // a crop keeps the line it starts at and stops before the line it ends at: start past the
    // last line with band in it, or end at it
    match last {
        Some(l) if from_end => l,
        Some(l) => l + 1,
        None => g,
    }
}

/// See [`past_wavy_gate`].
const WAVY_SHARE: f64 = 0.2;
const WAVY_SPREAD: f32 = 0.05;
const WAVY_MAX: f64 = 0.04;

/// Per channel, the mean and spread of the strip of lines `a..z` (rows when `rows`, else columns)
/// of `p` over `along`.
fn strip_stats(
    p: &Planes,
    rows: bool,
    a: usize,
    z: usize,
    along: std::ops::Range<usize>,
) -> [(f32, f32); 3] {
    std::array::from_fn(|k| {
        let c = &p.c[k];
        let (mut sum, mut sq, mut n) = (0.0f32, 0.0f32, 0usize);
        for line in a..z {
            for i in along.clone() {
                let v = if rows {
                    c[line * p.w + i]
                } else {
                    c[i * p.w + line]
                };
                sum += v;
                sq += v * v;
                n += 1;
            }
        }
        let n = n.max(1) as f32;
        let mean = sum / n;
        (mean, (sq / n - mean * mean).max(0.0).sqrt())
    })
}

/// Whether the strip cut off between edge `e` and the candidate `o` outside it is more of the
/// picture: as bright, as coloured and as busy as a strip of the same width just inside `e` (within
/// [`SAME_MEAN`] and [`SAME_SPREAD`] in every channel). A rebate, a mask, sprocket holes or the
/// band past a gate are not; a scene's straight line standing in for the frame's edge cuts off
/// more of the same scene.
fn picture_continues(
    p: &Planes,
    rows: bool,
    e: usize,
    o: usize,
    n: usize,
    along: std::ops::Range<usize>,
) -> bool {
    let from_end = o > e;
    let (a, z) = (e.min(o) + 2, e.max(o).saturating_sub(2));
    if z <= a + 1 {
        return false;
    }
    let width = z - a;
    let (ia, iz) = if from_end {
        (e.saturating_sub(2 + width), e.saturating_sub(2))
    } else {
        (e + 2, (e + 2 + width).min(n))
    };
    if iz <= ia {
        return false;
    }
    // a band (the dark band past a gate next to a dark scene, say) is never more picture
    if band_tone(p, rows, a, z, along.clone()).is_some() {
        return false;
    }
    let (out, inside) = (
        strip_stats(p, rows, a, z, along.clone()),
        strip_stats(p, rows, ia, iz, along),
    );
    out.iter()
        .zip(&inside)
        .all(|(o, i)| (o.0 - i.0).abs() <= SAME_MEAN && (o.1 - i.1).abs() <= SAME_SPREAD)
}

/// See [`picture_continues`].
const SAME_MEAN: f32 = 0.08;
const SAME_SPREAD: f32 = 0.06;

/// Moves each side of a format's box out past strips that are more of the picture
/// ([`picture_continues`]): to the next candidate outside, while the box keeps a film format's
/// shape. `sides` are the candidates of left, top, right, bottom.
fn keep_the_picture(p: &Planes, sides: &[Vec<usize>; 4], edges: &mut [usize; 4], format: Format) {
    let (w, h) = (p.w, p.h);
    let span = |n: usize| (n as f64 * SPAN.0) as usize..(n as f64 * SPAN.1) as usize;
    for k in 0..4 {
        let (n, rows, along) = if k % 2 == 0 {
            (w, false, span(h))
        } else {
            (h, true, span(w))
        };
        loop {
            let e = edges[k];
            let outward = |c: usize| if k < 2 { c < e } else { c > e };
            let Some(o) = sides[k]
                .iter()
                .copied()
                .filter(|&c| outward(c))
                .min_by_key(|&c| c.abs_diff(e))
            else {
                break;
            };
            let mut moved = *edges;
            moved[k] = o;
            let (bw, bh) = (
                moved[2] as f64 - moved[0] as f64,
                moved[3] as f64 - moved[1] as f64,
            );
            if format_of(bw, bh, format).is_none()
                || !picture_continues(p, rows, e, o, n, along.clone())
            {
                break;
            }
            *edges = moved;
        }
    }
}

/// The frame's edges in `p` (left, top, right, bottom; `None`: none found on that side): as found,
/// then stepped in past bands ([`past_bands`]); and the film format they make, if any. The edges as
/// found are the straight ones to measure a tilt on; a gate's own edge can be wavy.
fn find_edges(
    p: &Planes,
    opts: &Options,
) -> ([Option<usize>; 4], [Option<usize>; 4], Option<usize>) {
    let (rows, cols) = (score_rows(p, opts.band), score_cols(p, opts.band));
    let sides = [
        candidates(&cols, opts.band, false),
        candidates(&rows, opts.band, false),
        candidates(&cols, opts.band, true),
        candidates(&rows, opts.band, true),
    ];
    // four real edges in a film format's shape
    let (mut edges, format) = match format_box(&sides, p.w as f64, p.h as f64, opts.format) {
        Some((mut b, f)) => {
            keep_the_picture(p, &sides, &mut b, opts.format);
            (b.map(Some), Some(f))
        }
        // else side by side
        None => {
            let innermost = opts.format == Format::Any;
            let dims = [p.w, p.h, p.w, p.h];
            (
                std::array::from_fn(|k| side_edge(&sides[k], dims[k], k >= 2, innermost)),
                None,
            )
        }
    };
    let found = edges;
    past_bands(p, &rows, &cols, opts.band, &mut edges);
    (found, edges, format)
}

/// Finds the picture area in `rgb` (interleaved 8-bit RGB, `w × h`, rows top to bottom). `None`
/// when no side has a clear edge.
///
/// A frame that lies tilted on the scanner is measured, and the edges found again in a copy
/// turned straight: the box is then the crop in the crop's rotated frame, and `angle` its angle.
pub fn detect(rgb: &[u8], w: usize, h: usize, opts: &Options) -> Option<Detection> {
    if w < 16 || h < 16 || rgb.len() < w * h * 3 {
        return None;
    }
    let p = Planes::from_rgb8(rgb, w, h);
    let (found, mut edges, mut format) = find_edges(&p, opts);
    if edges.iter().all(Option::is_none) {
        return None;
    }
    // the frame turned clockwise by φ is straightened by turning it back: a crop angle of −φ
    // the picture's own edges first (inside a band, the gate's: the picture can sit skewed in a
    // film strip that lies straight), the edges as found when those give no tilt
    let tilt = match frame_tilt(&p, &edges) {
        0.0 => frame_tilt(&p, &found),
        t => t,
    };
    let angle = if tilt == 0.0 { 0.0 } else { -tilt };
    if angle != 0.0 {
        (_, edges, format) = find_edges(&p.rotated(angle), opts);
        if edges.iter().all(Option::is_none) {
            return None;
        }
    }
    let (wf, hf) = (w as f64, h as f64);
    let inset = opts.inset.max(0.0);
    let at = |e: Option<usize>, dim: f64, edge: f64, inwards: f64| {
        e.map_or(edge, |i| i as f64 / dim + inwards * inset)
            .clamp(0.0, 1.0)
    };
    let rect = [
        at(edges[0], wf, 0.0, 1.0),
        at(edges[1], hf, 0.0, 1.0),
        at(edges[2], wf, 1.0, -1.0),
        at(edges[3], hf, 1.0, -1.0),
    ];
    if rect[2] - rect[0] < MIN_SIDE || rect[3] - rect[1] < MIN_SIDE {
        return None;
    }
    Some(Detection {
        rect,
        found: edges.map(|e| e.is_some()),
        format,
        angle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w × h` image: `outside` everywhere, `inside` in the box, plus some texture in the box.
    fn framed(w: usize, h: usize, b: [usize; 4], outside: [u8; 3], inside: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let px = if x >= b[0] && x < b[2] && y >= b[1] && y < b[3] {
                    // texture: a picture is never flat
                    let t = ((x * 7 + y * 13) % 23) as u8;
                    inside.map(|c| c.saturating_add(t))
                } else {
                    outside
                };
                v.extend_from_slice(&px);
            }
        }
        v
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn no_inset() -> Options {
        Options {
            inset: 0.0,
            ..Options::default()
        }
    }

    #[test]
    fn a_picture_on_a_flat_border_is_found_on_every_side() {
        let (w, h) = (300, 200);
        let img = framed(w, h, [30, 20, 270, 180], [10, 10, 10], [140, 120, 100]);
        let d = detect(&img, w, h, &no_inset()).unwrap();
        assert_eq!(d.found, [true; 4]);
        assert_eq!(d.format, Some(0), "240 × 160 is 3:2");
        let want = [30.0 / 300.0, 20.0 / 200.0, 270.0 / 300.0, 180.0 / 200.0];
        for (got, want) in d.rect.iter().zip(want) {
            assert!(close(*got, want, 0.01), "{:?} vs {want:?}", d.rect);
        }
    }

    #[test]
    fn the_innermost_edge_wins_over_a_stronger_outer_one() {
        // white paper, a black rebate, then a mid-grey picture: the paper/rebate edge is far
        // stronger, but the picture edge is the crop
        let (w, h) = (300, 200);
        let mut img = framed(w, h, [20, 15, 280, 185], [255, 255, 255], [0, 0, 0]);
        let inner = framed(w, h, [50, 35, 250, 165], [0, 0, 0], [110, 100, 90]);
        for y in 15..185 {
            for x in 20..280 {
                let i = 3 * (y * w + x);
                img[i..i + 3].copy_from_slice(&inner[i..i + 3]);
            }
        }
        let d = detect(&img, w, h, &no_inset()).unwrap();
        assert!(
            close(d.rect[0], 50.0 / 300.0, 0.01) && close(d.rect[2], 250.0 / 300.0, 0.01),
            "{:?}",
            d.rect
        );
        assert!(
            close(d.rect[1], 35.0 / 200.0, 0.01) && close(d.rect[3], 165.0 / 200.0, 0.01),
            "{:?}",
            d.rect
        );
    }

    #[test]
    fn a_straight_line_in_the_scene_loses_to_the_frame_s_shape() {
        // a 4:3 frame (240 × 180) whose picture has a full-width step 40 rows in, like the base
        // of a building: the innermost edge would be the step, the format says it's the frame
        let (w, h) = (300, 220);
        let mut img = framed(w, h, [30, 20, 270, 200], [5, 5, 5], [90, 90, 90]);
        for y in 60..200 {
            for x in 30..270 {
                let i = 3 * (y * w + x);
                for c in &mut img[i..i + 3] {
                    *c = c.saturating_add(110);
                }
            }
        }
        let d = detect(&img, w, h, &no_inset()).unwrap();
        assert!(
            close(d.rect[1], 20.0 / 220.0, 0.01),
            "the frame's top, not the step: {:?}",
            d.rect
        );
        assert_eq!(d.format, Some(1));
        // asked for no shape, the innermost edge wins
        let d = detect(
            &img,
            w,
            h,
            &Options {
                format: Format::Any,
                ..no_inset()
            },
        )
        .unwrap();
        assert!(close(d.rect[1], 60.0 / 220.0, 0.01), "{:?}", d.rect);
    }

    #[test]
    fn a_faint_gate_shows_once_the_channels_are_stretched() {
        // a thin negative inside a black holder: a white rebate, then a picture only a little
        // darker on average (a tenth of the holder's edge); the gate is that faint step
        let (w, h) = (300, 200);
        let mut img = framed(w, h, [10, 0, 290, 200], [0, 0, 0], [235, 225, 215]);
        let pic = framed(w, h, [24, 0, 276, 200], [235, 225, 215], [200, 190, 180]);
        for y in 0..200 {
            for x in 10..290 {
                let i = 3 * (y * w + x);
                img[i..i + 3].copy_from_slice(&pic[i..i + 3]);
            }
        }
        let d = detect(&img, w, h, &no_inset()).unwrap();
        assert!(
            close(d.rect[0], 24.0 / 300.0, 0.01) && close(d.rect[2], 276.0 / 300.0, 0.01),
            "{:?}",
            d.rect
        );
    }

    #[test]
    fn a_picture_with_no_border_is_left_alone() {
        let (w, h) = (300, 200);
        let img = framed(w, h, [0, 0, 300, 200], [0, 0, 0], [100, 100, 100]);
        assert_eq!(detect(&img, w, h, &Options::default()), None);
    }

    /// A 3:2 picture on white paper, turned by `tilt` degrees clockwise about the image centre:
    /// `w × h` RGB.
    fn tilted_frame(w: usize, h: usize, tilt: f64) -> Vec<u8> {
        let (s, c) = tilt.to_radians().sin_cos();
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let (fw, fh) = (0.8 * w as f64, 0.8 * w as f64 / 1.5);
        let mut v = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                // back into the untilted frame
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                let (ux, uy) = (c * dx + s * dy, -s * dx + c * dy);
                let inside = ux.abs() < fw / 2.0 && uy.abs() < fh / 2.0;
                let t = ((x * 7 + y * 13) % 23) as u8;
                v.extend_from_slice(&if inside {
                    [60 + t, 50 + t, 40 + t]
                } else {
                    [250, 250, 250]
                });
            }
        }
        v
    }

    #[test]
    fn a_tilted_frame_is_straightened_and_cropped_to_its_edges() {
        let (w, h) = (512, 400);
        for tilt in [-1.0, -0.4, 0.5, 1.5] {
            let img = tilted_frame(w, h, tilt);
            let d = detect(&img, w, h, &no_inset()).unwrap();
            assert!(
                (d.angle + tilt).abs() < 0.08,
                "tilt {tilt}: angle {}",
                d.angle
            );
            // straightened, the frame is centred, 0.8 of the width and 3:2
            let (fw, fh) = (0.8, 0.8 * w as f64 / 1.5 / h as f64);
            let want = [
                0.5 - fw / 2.0,
                0.5 - fh / 2.0,
                0.5 + fw / 2.0,
                0.5 + fh / 2.0,
            ];
            for (got, want) in d.rect.iter().zip(want) {
                assert!(
                    close(*got, want, 0.006),
                    "tilt {tilt}: {:?} vs {want:?}",
                    d.rect
                );
            }
        }
        // a frame that lies straight isn't turned
        let d = detect(&tilted_frame(w, h, 0.0), w, h, &no_inset()).unwrap();
        assert_eq!(d.angle, 0.0);
    }

    #[test]
    fn the_black_band_past_the_gate_is_cut_away_even_where_the_picture_is_dark() {
        // white paper, then the film's black band, then the picture, half of it dark against the
        // band, and the band a little wider on the right (a wavy gate): no tilt, and the crop
        // inside the band without cutting deep into the picture
        let (w, h) = (480, 330);
        let band = |x: usize, y: usize| -> bool {
            let extra = x / 160; // 0..2 lines more band towards the right
            x < 30 || x >= w - 30 || y < 20 + extra || y >= h - 20
        };
        let mut img = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let px = if x < 12 || x >= w - 12 || y < 6 || y >= h - 6 {
                    [250, 250, 250]
                } else if band(x, y) {
                    [8, 6, 5]
                } else if (x / 24 + y / 24) % 2 == 0 {
                    // dark patches in the picture, against the band too
                    [30 + ((x * 7 + y * 13) % 23) as u8, 25, 22]
                } else {
                    [150 + ((x * 7 + y * 13) % 23) as u8, 130, 110]
                };
                img.extend_from_slice(&px);
            }
        }
        let d = detect(&img, w, h, &no_inset()).unwrap();
        assert_eq!(d.angle, 0.0, "a wavy gate is not a tilt");
        let [x0, y0, x1, y1] = d.rect.map(|v| v * 1000.0);
        // inside the band on every side (within a line: the edge's blur)
        assert!(
            x0 >= 30.0 / 480.0 * 1000.0 - 3.0 && x1 <= 450.0 / 480.0 * 1000.0 + 3.0,
            "{:?}",
            d.rect
        );
        assert!(
            y0 >= 20.0 / 330.0 * 1000.0 - 3.0 && y1 <= 310.0 / 330.0 * 1000.0 + 3.0,
            "{:?}",
            d.rect
        );
        // and not much further in
        assert!(
            x0 <= 80.0 && y0 <= 100.0 && x1 >= 920.0 && y1 >= 920.0,
            "{:?}",
            d.rect
        );
    }

    #[test]
    fn tiny_or_short_inputs_are_refused() {
        assert_eq!(detect(&[0; 10], 2, 2, &Options::default()), None);
        assert_eq!(detect(&[0; 100], 100, 100, &Options::default()), None);
    }
}
