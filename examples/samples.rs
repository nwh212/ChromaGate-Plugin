//! Runs the detector on image files, as the plug-in sees them (downsized to its long edge), and
//! writes each with the crop drawn on it next to the original as `<name>.crop.png`.
//!
//! cargo run --example samples -- a.webp b.jpg …

use chromagate::detect::{Options, detect};

fn main() {
    for path in std::env::args().skip(1) {
        let img = image::open(&path)
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .to_rgb8();
        let edge: u32 = std::env::var("EDGE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(512);
        let s = (edge as f64 / img.width().max(img.height()) as f64).min(1.0);
        let small = image::imageops::resize(
            &img,
            ((img.width() as f64 * s).round() as u32).max(1),
            ((img.height() as f64 * s).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
        let d = detect(
            small.as_raw(),
            small.width() as usize,
            small.height() as usize,
            &Options::default(),
        );
        println!("{path}: {d:?}");
        let Some(d) = d else { continue };
        let mut out = img.clone();
        let (w, h) = (out.width() as f64, out.height() as f64);
        let [x0, y0, x1, y1] =
            [d.rect[0] * w, d.rect[1] * h, d.rect[2] * w, d.rect[3] * h].map(|v| v as i64);
        let t = (out.width().max(out.height()) / 300).max(2) as i64;
        for y in 0..out.height() as i64 {
            for x in 0..out.width() as i64 {
                let inside = x >= x0 - t && x <= x1 + t && y >= y0 - t && y <= y1 + t;
                let core = x > x0 && x < x1 && y > y0 && y < y1;
                if inside && !core {
                    out.put_pixel(x as u32, y as u32, image::Rgb([255, 0, 255]));
                }
            }
        }
        let dest = format!(
            "{}.crop.png",
            path.rsplit_once('.').map_or(path.as_str(), |(a, _)| a)
        );
        let k = 900.0 / out.width().max(out.height()) as f64;
        image::imageops::thumbnail(
            &out,
            (out.width() as f64 * k) as u32,
            (out.height() as f64 * k) as u32,
        )
        .save(&dest)
        .unwrap();
    }
}
