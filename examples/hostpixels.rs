//! Runs the detector on pixels exactly as Chromaforge's `photo.pixels` gave them: each argument is
//! a `<name>.rgb` file (interleaved 8-bit RGB) next to a `<name>.dim` file ("width height").
//!
//! cargo run --example hostpixels -- dir/*.rgb

use chromagate::detect::{Options, detect};

fn main() {
    for path in std::env::args().skip(1) {
        let rgb = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let base = path.strip_suffix(".rgb").unwrap_or(&path);
        let dim = std::fs::read_to_string(format!("{base}.dim")).unwrap();
        let (w, h) = dim.trim().split_once(' ').unwrap();
        let (w, h): (usize, usize) = (w.parse().unwrap(), h.parse().unwrap());
        println!("{base}: {:?}", detect(&rgb, w, h, &Options::default()));
    }
}
