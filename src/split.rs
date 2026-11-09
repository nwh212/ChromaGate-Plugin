//! Splits a gate that holds several exposures (two half-frames in a 35 mm gate, say) into one
//! rect per exposure. No host calls; its tests run natively.
//!
//! [`crate::detect::detect`] finds the gate and its straighten angle. Inside the gate, the
//! exposures lie side by side along its long side, with a strip of unexposed film between each
//! pair: black on a positive or a converted negative, film base on a raw negative. Whatever its
//! tone, the strip is one tone from top to bottom, while a line through a picture never is. So
//! every line across the long side is scored by how much it varies along its length, and the most
//! uniform run of lines near each expected place (`k / n` of the way) is a divider. With no such run
//! (the exposures touch or overlap), the gate is split evenly there.

/// What [`split`] found: one rect per exposure, `[x0, y0, x1, y1]` normalized in the frame turned
/// by the gate's angle (as Chromaforge's crop is), in order along the long side, left to right or
/// top to bottom; and, per divider, whether a strip between the exposures was seen (`false`: split
/// evenly there).
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub frames: Vec<[f64; 4]>,
    pub found: Vec<bool>,
}

/// How far either side of its expected place a divider is looked for, as a fraction of the gate's
/// long side.
const SEARCH: f64 = 0.12;
/// Part of each line scored: its middle, away from the gate's own edges.
const SPAN: (f64, f64) = (0.10, 0.90);
/// A line is uniform when it varies (standard deviation, 0..255) by less than this share of the
/// gate's median line, and less than [`MAX_STD`]; [`MIN_STD`] keeps grain from making every line
/// of a flat gate "busy".
const REL_STD: f64 = 0.35;
const MIN_STD: f64 = 4.0;
const MAX_STD: f64 = 18.0;
/// Fewest uniform lines in a row that make a divider.
const MIN_RUN: usize = 2;
/// Smallest share of the gate's long side one exposure may take.
const MIN_FRAME: f64 = 0.15;

/// The gate `gate` (normalized, in the frame turned by `angle` degrees clockwise, as
/// [`crate::detect::Detection`] gives it) of a `w × h` photo `rgb` (interleaved 8-bit RGB), split
/// into `n` exposures along its long side, each moved in by `inset` of its own size on every side.
/// `None` when `n < 2`, the pixels are too small, or the gate leaves too little room per exposure.
pub fn split(
    rgb: &[u8],
    w: usize,
    h: usize,
    gate: [f64; 4],
    angle: f64,
    n: usize,
    inset: f64,
) -> Option<Split> {
    if n < 2 || w < 16 || h < 16 || rgb.len() < w * h * 3 {
        return None;
    }
    let [gx0, gy0, gx1, gy1] = gate;
    if !(gx0 < gx1 && gy0 < gy1) {
        return None;
    }
    let (wf, hf) = (w as f64, h as f64);
    // the gate in pixels of the turned frame
    let (px0, py0) = (
        (gx0 * wf).round().max(0.0) as usize,
        (gy0 * hf).round().max(0.0) as usize,
    );
    let (px1, py1) = (
        (gx1 * wf).round().min(wf) as usize,
        (gy1 * hf).round().min(hf) as usize,
    );
    let (gw, gh) = (px1.saturating_sub(px0), py1.saturating_sub(py0));
    // the exposures lie along the gate's long side, in its own pixels
    let along_x = gw as f64 >= gh as f64;
    let (len, across) = if along_x { (gw, gh) } else { (gh, gw) };
    if (len as f64) < n as f64 * 8.0 || across < 8 {
        return None;
    }
    let gate_px = Gate::sample(rgb, w, h, angle, px0, py0, gw, gh);
    let stds: Vec<f64> = (0..len).map(|i| gate_px.line_std(i, along_x)).collect();
    let mut sorted = stds.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let limit = (median * REL_STD).clamp(MIN_STD, MAX_STD);
    let uniform: Vec<bool> = stds.iter().map(|s| *s < limit).collect();

    // edges along the long side, in the gate's pixels: 0, then each divider's two sides, then len
    let mut cuts = vec![0usize];
    let mut found = Vec::with_capacity(n - 1);
    let min_frame = ((len as f64) * MIN_FRAME).ceil() as usize;
    for k in 1..n {
        let target = len as f64 * k as f64 / n as f64;
        let reach = len as f64 * SEARCH;
        let lo = ((target - reach).max(0.0) as usize).max(cuts[cuts.len() - 1] + min_frame);
        let hi = ((target + reach) as usize).min(len);
        match nearest_run(&uniform, &stds, lo, hi, target) {
            Some((a, b)) => {
                cuts.extend([a, b]);
                found.push(true);
            }
            None => {
                let t = (target.round() as usize).max(cuts[cuts.len() - 1] + 1);
                cuts.extend([t, t]);
                found.push(false);
            }
        }
    }
    cuts.push(len);

    let inset = inset.clamp(0.0, 0.2);
    let frames = cuts
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[a, b]| {
            let (a, b) = (a as f64, b as f64);
            let pad_along = (b - a) * inset;
            let pad_across = across as f64 * inset;
            let (a, b) = (a + pad_along, b - pad_along);
            let (s0, s1) = (pad_across, across as f64 - pad_across);
            let (x0, y0, x1, y1) = if along_x {
                (a, s0, b, s1)
            } else {
                (s0, a, s1, b)
            };
            [
                (px0 as f64 + x0) / wf,
                (py0 as f64 + y0) / hf,
                (px0 as f64 + x1) / wf,
                (py0 as f64 + y1) / hf,
            ]
            .map(|v| v.clamp(0.0, 1.0))
        })
        .collect::<Vec<_>>();
    if frames.iter().any(|f| f[2] <= f[0] || f[3] <= f[1]) {
        return None;
    }
    Some(Split { frames, found })
}

/// The run of uniform lines in `lo..hi` whose middle is nearest `target` (at least [`MIN_RUN`]
/// lines), widened over its soft sides: neighbours that vary less than twice the run's own mean.
fn nearest_run(
    uniform: &[bool],
    stds: &[f64],
    lo: usize,
    hi: usize,
    target: f64,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize, f64)> = None;
    let mut i = lo;
    while i < hi {
        if !uniform[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < hi && uniform[i] {
            i += 1;
        }
        if i - start >= MIN_RUN {
            let d = ((start + i) as f64 / 2.0 - target).abs();
            if best.is_none_or(|(_, _, bd)| d < bd) {
                best = Some((start, i, d));
            }
        }
    }
    let (mut a, mut b, _) = best?;
    let mean = stds[a..b].iter().sum::<f64>() / (b - a) as f64;
    let soft = (mean * 2.0).max(MIN_STD);
    while a > lo && stds[a - 1] < soft {
        a -= 1;
    }
    while b < hi && stds[b] < soft {
        b += 1;
    }
    Some((a, b))
}

/// The gate's pixels in the frame turned by the crop's angle, as 0..255 floats per channel.
struct Gate {
    w: usize,
    h: usize,
    c: [Vec<f32>; 3],
}

impl Gate {
    /// The `gw × gh` pixels at `(x0, y0)` of `rgb` turned by `angle` degrees clockwise about the
    /// photo's centre, as Chromaforge's crop turns it (and [`crate::detect`] does). Bilinear; what
    /// falls beyond the photo repeats its edge.
    #[allow(clippy::too_many_arguments)]
    fn sample(
        rgb: &[u8],
        w: usize,
        h: usize,
        angle: f64,
        x0: usize,
        y0: usize,
        gw: usize,
        gh: usize,
    ) -> Gate {
        let (s, c) = angle.to_radians().sin_cos();
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let (wm, hm) = ((w - 1) as f64 - 1e-3, (h - 1) as f64 - 1e-3);
        let mut out: [Vec<f32>; 3] = std::array::from_fn(|_| vec![0.0f32; gw * gh]);
        let at = |x: usize, y: usize, ch: usize| f32::from(rgb[3 * (y * w + x) + ch]);
        for y in 0..gh {
            let dy = (y0 + y) as f64 + 0.5 - cy;
            for x in 0..gw {
                let dx = (x0 + x) as f64 + 0.5 - cx;
                let px = (c * dx + s * dy + cx - 0.5).clamp(0.0, wm);
                let py = (-s * dx + c * dy + cy - 0.5).clamp(0.0, hm);
                let (ix, iy) = (px as usize, py as usize);
                let (fx, fy) = ((px - ix as f64) as f32, (py - iy as f64) as f32);
                for (ch, o) in out.iter_mut().enumerate() {
                    let top = at(ix, iy, ch) + (at(ix + 1, iy, ch) - at(ix, iy, ch)) * fx;
                    let bot =
                        at(ix, iy + 1, ch) + (at(ix + 1, iy + 1, ch) - at(ix, iy + 1, ch)) * fx;
                    o[y * gw + x] = top + (bot - top) * fy;
                }
            }
        }
        Gate {
            w: gw,
            h: gh,
            c: out,
        }
    }

    /// How much line `i` across the long side varies along its middle ([`SPAN`]): the standard
    /// deviation of each channel, 0..255, in the channel where it's largest.
    fn line_std(&self, i: usize, along_x: bool) -> f64 {
        let n = if along_x { self.h } else { self.w };
        let (a, b) = (
            (n as f64 * SPAN.0) as usize,
            ((n as f64 * SPAN.1) as usize).max(1),
        );
        let mut worst = 0.0f64;
        for ch in &self.c {
            let (mut sum, mut sq) = (0.0f64, 0.0f64);
            for j in a..b {
                let v = f64::from(if along_x {
                    ch[j * self.w + i]
                } else {
                    ch[i * self.w + j]
                });
                sum += v;
                sq += v * v;
            }
            let k = (b - a) as f64;
            let mean = sum / k;
            worst = worst.max((sq / k - mean * mean).max(0.0).sqrt());
        }
        worst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w × h` scan: `border` around the gate `g`, `n` textured exposures across the gate with a
    /// `gap`-pixel strip of `strip` between each pair.
    fn scan(
        w: usize,
        h: usize,
        g: [usize; 4],
        n: usize,
        gap: usize,
        border: [u8; 3],
        strip: [u8; 3],
    ) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 3];
        let len = g[2] - g[0];
        let frame = (len - gap * (n - 1)) / n;
        for y in 0..h {
            for x in 0..w {
                let px = if x < g[0] || x >= g[2] || y < g[1] || y >= g[3] {
                    border
                } else {
                    let off = x - g[0];
                    let in_gap = (off % (frame + gap)) >= frame && off < len - frame / 2;
                    if in_gap {
                        strip
                    } else {
                        // a picture: never one tone down a column
                        let t = ((x * 7 + y * 13) % 41) as u8 + ((y * 3) % 90) as u8;
                        [60 + t, 80 + t / 2, 100u8.saturating_add(t)]
                    }
                };
                v[3 * (y * w + x)..3 * (y * w + x) + 3].copy_from_slice(&px);
            }
        }
        v
    }

    fn norm(g: [usize; 4], w: usize, h: usize) -> [f64; 4] {
        [
            g[0] as f64 / w as f64,
            g[1] as f64 / h as f64,
            g[2] as f64 / w as f64,
            g[3] as f64 / h as f64,
        ]
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn two_half_frames_split_at_the_black_strip() {
        let (w, h) = (360, 240);
        let g = [20, 20, 340, 220];
        let img = scan(w, h, g, 2, 12, [0, 0, 0], [5, 5, 5]);
        let s = split(&img, w, h, norm(g, w, h), 0.0, 2, 0.0).unwrap();
        assert_eq!(s.found, vec![true]);
        // frame 154 px, then 12 px of strip
        let (f1, f2) = (s.frames[0], s.frames[1]);
        assert!(
            close(f1[0], 20.0 / 360.0, 0.004) && close(f1[2], 174.0 / 360.0, 0.01),
            "{f1:?}"
        );
        assert!(
            close(f2[0], 186.0 / 360.0, 0.01) && close(f2[2], 340.0 / 360.0, 0.004),
            "{f2:?}"
        );
        assert!(close(f1[1], 20.0 / 240.0, 0.004) && close(f1[3], 220.0 / 240.0, 0.004));
    }

    #[test]
    fn a_raw_negative_s_light_film_base_strip_is_a_divider_too() {
        let (w, h) = (360, 240);
        let g = [10, 10, 350, 230];
        let img = scan(w, h, g, 2, 10, [20, 10, 8], [230, 160, 130]);
        let s = split(&img, w, h, norm(g, w, h), 0.0, 2, 0.0).unwrap();
        assert_eq!(s.found, vec![true]);
        let mid = (s.frames[0][2] + s.frames[1][0]) / 2.0;
        assert!(close(mid, 180.0 / 360.0, 0.01), "{s:?}");
    }

    #[test]
    fn exposures_that_touch_are_split_evenly() {
        let (w, h) = (300, 200);
        let g = [0, 0, 300, 200];
        let img = scan(w, h, g, 2, 0, [0, 0, 0], [0, 0, 0]);
        let s = split(&img, w, h, norm(g, w, h), 0.0, 2, 0.0).unwrap();
        assert_eq!(s.found, vec![false]);
        assert!(
            close(s.frames[0][2], 0.5, 0.005) && close(s.frames[1][0], 0.5, 0.005),
            "{s:?}"
        );
    }

    #[test]
    fn three_exposures_give_three_frames_in_order() {
        let (w, h) = (390, 200);
        let g = [15, 10, 375, 190];
        let img = scan(w, h, g, 3, 9, [0, 0, 0], [0, 0, 0]);
        let s = split(&img, w, h, norm(g, w, h), 0.0, 3, 0.0).unwrap();
        assert_eq!(s.found, vec![true, true]);
        assert_eq!(s.frames.len(), 3);
        assert!(s.frames.windows(2).all(|p| p[0][2] < p[1][0]), "{s:?}");
    }

    #[test]
    fn a_tall_gate_splits_top_to_bottom() {
        // the same scan turned a quarter: exposures stacked
        let (w, h) = (240, 360);
        let wide = scan(h, w, [20, 20, 340, 220], 2, 12, [0, 0, 0], [5, 5, 5]);
        let mut img = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                // (x, y) here is (y, x) in the wide scan
                let s = 3 * (x * h + y);
                img[3 * (y * w + x)..3 * (y * w + x) + 3].copy_from_slice(&wide[s..s + 3]);
            }
        }
        let s = split(&img, w, h, norm([20, 20, 220, 340], w, h), 0.0, 2, 0.0).unwrap();
        assert_eq!(s.found, vec![true]);
        assert!(s.frames[0][3] < s.frames[1][1], "{s:?}");
        assert!(close(s.frames[0][0], 20.0 / 240.0, 0.004));
    }

    #[test]
    fn the_inset_moves_every_side_in_by_its_own_share() {
        let (w, h) = (360, 240);
        let g = [20, 20, 340, 220];
        let img = scan(w, h, g, 2, 12, [0, 0, 0], [5, 5, 5]);
        let flush = split(&img, w, h, norm(g, w, h), 0.0, 2, 0.0).unwrap();
        let trimmed = split(&img, w, h, norm(g, w, h), 0.0, 2, 0.02).unwrap();
        for (a, b) in flush.frames.iter().zip(&trimmed.frames) {
            assert!(
                b[0] > a[0] && b[1] > a[1] && b[2] < a[2] && b[3] < a[3],
                "{a:?} {b:?}"
            );
        }
    }

    #[test]
    fn a_tilted_scan_splits_in_the_turned_frame() {
        // the half-frame pair turned 2° clockwise; straightened with an angle of −2°
        let (w, h) = (360, 240);
        let g = [30, 30, 330, 210];
        let flat = scan(w, h, g, 2, 12, [0, 0, 0], [5, 5, 5]);
        let turned = Gate::sample(&flat, w, h, -2.0, 0, 0, w, h);
        let img: Vec<u8> = (0..w * h)
            .flat_map(|i| turned.c.iter().map(move |c| c[i].round() as u8))
            .collect();
        let s = split(&img, w, h, norm(g, w, h), 2.0, 2, 0.0).unwrap();
        assert_eq!(s.found, vec![true]);
        let mid = (s.frames[0][2] + s.frames[1][0]) / 2.0;
        assert!(close(mid, 180.0 / 360.0, 0.01), "{s:?}");
    }

    #[test]
    fn nonsense_gives_nothing() {
        let img = vec![0u8; 100 * 100 * 3];
        assert!(split(&img, 100, 100, [0.0, 0.0, 1.0, 1.0], 0.0, 1, 0.0).is_none());
        assert!(split(&img, 100, 100, [0.6, 0.0, 0.4, 1.0], 0.0, 2, 0.0).is_none());
        assert!(split(&img[..10], 100, 100, [0.0, 0.0, 1.0, 1.0], 0.0, 2, 0.0).is_none());
    }
}
