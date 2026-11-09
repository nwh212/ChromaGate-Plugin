<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/logo-dark.svg">
    <img alt="ChromaGate" src="brand/logo-light.svg" width="560">
  </picture>
</p>

<p align="center">
  <b>Finds the film gate in your scans, then crops and levels every frame.</b><br>
  A plug-in for <a href="https://github.com/emmertex/chromaforge">Chromaforge</a>, from the maker of ChromaLab.
</p>

---

Select your scanned frames, run **File ▸ Plug-ins ▸ ChromaGate ▸ Auto-Crop Film Frames…**, and each
photo gets a crop to the picture inside its film gate, leveled when the film lay skewed on the
scanner. It's an ordinary crop: undo it, or adjust it in the crop tool when it's off.

It works on frames in any state:

- **Raw negatives**, with their orange rebate, edge print and sprocket holes, in a black holder
- **Converted negatives**, with the black band past the gate, scanned on white or on a light table
- **Slides** in their mounts, with the thin black mask
- **Thin negatives** whose gate barely shows

## Requirements

A Chromaforge build with the host features `pixels`, `crop` and `virtualCopy`, the plug-in API that
lets a plug-in look at a photo, set its crop and make a virtual copy of it. A build without them
refuses to install ChromaGate and says which feature is missing.

These features are proposed upstream in
[emmertex/chromaforge#891](https://github.com/emmertex/chromaforge/pull/891). Until it's merged,
ChromaGate needs a Chromaforge build that includes that pull request.

## Install

1. Download `chromagate-<version>.wasm` from [Releases](https://github.com/nwh212/ChromaGate-Plugin/releases)
   (or build it, below).
2. In Chromaforge: **File ▸ Plug-ins ▸ Plug-in Manager… ▸ Install from File…**, or drop the file on
   the window.
3. Tick the permissions:
   - *Look at downsized copies of photos*: ChromaGate looks at a 512-pixel copy of each photo.
   - *Change the crop of photos*: to set the crop it found.
   - *Make virtual copies of photos*: only *Split Half-Frame Scans* uses it, for the second (third,
     fourth) exposure in a scan.

ChromaGate asks for nothing else: no network, no files, no catalog.

## Options

- **Frame format**: *Auto* (default) looks for the shape of a real film format: 35mm / 6×9 (3:2),
  6×4.5 / half-frame (4:3), 6×6 (1:1), 6×7 / 4×5. Pick one to hold a whole roll to it, or *Any
  shape* to take the innermost edge on each side.
- **Trim inside the frame edge**: moves each edge inwards (0.4 % of the side by default) to hide the
  soft line a frame edge leaves. Lower it to keep more of the picture; below about 0.2 % a sliver of
  the border can show.

Each photo's result (crop, straighten angle, edges found, film format) is in **Plug-in Manager ▸
ChromaGate ▸ Log**.

## Half-frame scans

A half-frame camera puts two exposures in one 35 mm frame, so a 35 mm gate scan holds a pair. Select
the scans and run **File ▸ Plug-ins ▸ ChromaGate ▸ Split Half-Frame Scans…**. ChromaGate finds the
gate and levels it as Auto-Crop does, finds the strip of unexposed film between the exposures, then:

- crops the original to one exposure, and
- makes a virtual copy (named *Exposure 2*, stacked with the original) cropped to the other.

Both crops share the straighten angle, and both are ordinary crops you can adjust or undo.

- **Exposures in each scan**: 2 (half-frame, the default), or 3 or 4 for smaller formats laid
  along the strip.
- **The original keeps**: the first exposure (left, or top on a tall scan) or the last. Copies take
  the others in order along the film.
- **Gate shape**: *35mm (3:2)* by default, the shape of a 35 mm gate around a half-frame pair. *Auto*
  and *Any shape* work as in Auto-Crop, for other gates.
- **Trim inside each exposure**: moves every side of each exposure inwards (1 % of its side by
  default), to clear the soft edge of the strip between them.

The strip can be any tone: black on a positive or a converted negative, the orange film base on a
raw negative. What makes it a strip is that it's one tone from top to bottom, which a line through
a picture never is. When the exposures touch, with no strip, the gate is split evenly and the
notice says so.

Running it again on the same scans re-crops the copies it made earlier instead of making new ones
(a copy you removed is made again), and a copy it made is never split itself.

## How it finds the gate

The picture is bounded by one long, straight edge that runs across nearly the whole frame. For each
side, ChromaGate scores every row (or column) in the outer 30 % of the image by how *coherent* the
edge across it is: the mean gradient in the colour channel where it's strongest, and how much of
the line pushes the same way. A row through a scene has edges of both signs; a row through sprocket
holes alternates and cancels; a frame edge is one sign end to end. Each channel is first stretched
to the image's own range, so a thin negative's faint gate still gives an edge.

A scene can hold a long straight line too (a wall, the base of a building), so the four edges chosen
are the ones that make a film format's shape. Among boxes of that shape, the smaller one wins
unless the strip it would cut off looks like more of the same picture.

Around most frames lies a band: the black film past the gate with its edge print, a slide's mask, a
rebate. Its inner edge is often soft, wavy, or dark against the picture, so a side steps inward past
a strip that is nearly all one dark or light tone when the picture beyond doesn't look like the
band, and on past the deepest point of a wavy gate.

A skewed frame is leveled: each edge is measured in 16 pieces along it, the picture's own edges
first (a picture can sit skewed inside a film strip that lies straight). An edge that measures
square is usually the scanner's mask, square to the sensor while the film in it isn't, so it doesn't
overrule a tilted one. The edges are found again in a straightened copy, and the crop is set with
that straighten angle (up to 5°).

## Limits

- **6 photos per run.** Each photo takes a share of the call's instruction budget (more when it's
  skewed). With more selected, run it again with the same selection: it carries on where it
  stopped. Split Half-Frame Scans does 4 scans per run, the same way.
- A side with no visible gate (a scan framed inside the picture) stays at the scan's edge; the
  notice says how many photos had clear edges on only some sides.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
# target/wasm32-unknown-unknown/release/chromagate.wasm
```

`cargo test` runs the detector's tests natively. `cargo run --release --example samples -- a.tif
b.jpg …` runs it on image files and writes each with the crop drawn on it as `<name>.crop.png`;
`--example hostpixels` does the same on pixels saved from Chromaforge's `photo.pixels`.

## Working on it

Checks before every commit (CI runs the same): `cargo fmt --check`, `cargo clippy --release
--all-targets -- -D warnings`, `cargo test --release`, and the wasm build above.


- `src/detect.rs`: the detector (no host calls; its tests run natively).
- `src/split.rs`: splits a gate into its exposures, in the straightened frame (no host calls; its
  tests run natively).
- `src/lib.rs`: the plug-in: manifest, the commands, the host calls.
- `brand/`: the logo and icon (original work, MIT), written by `python tools/brand.py`. The
  manifest embeds them: the icon, and the wordmark for dark and light themes as the header picture.
- **Releasing:** bump `version` in `Cargo.toml`, add a `CHANGELOG.md` entry, push, then publish a
  GitHub release tagged `vX.Y.Z`. CI checks the tag against the version, builds the plug-in and
  attaches `chromagate-vX.Y.Z.wasm`, with the changelog's notes if the release has none.
- The id, `io.github.nwh212.chromagate`, never changes: an install with another id is another
  plug-in. The version comes from `Cargo.toml` only.

## Licence

MIT. See [LICENSE](LICENSE).
