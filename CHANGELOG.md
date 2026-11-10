# Changelog

## 0.4.1 (2026-11-09)

- The Plug-in Manager's About tab links to ChromaGate's homepage, source code and changelog, shows
  its licence (MIT), and offers Report a Problem (this repository's issues). Needs a Chromaforge
  build with emmertex/chromaforge#1004; older builds ignore the links.

## 0.4.0 (2026-11-08)

First release as ChromaGate (developed as "Film Auto-Crop").

- Finds the frame on raw negatives, converted negatives, slides in mounts and thin negatives, and
  sets it as an ordinary, editable crop.
- Chooses the four edges that make a real film format's shape (3:2, 6×4.5 / 4:3, 6×6, 6×7 / 4×5),
  so a straight line in the scene doesn't stand in for the frame's edge; or hold a roll to one
  format, or take any shape.
- Steps past the black band beyond the gate (edge print and all), slide masks and rebates, and past
  the deepest point of a wavy gate.
- Levels skewed frames (up to 5°) from the picture's own edges, so a picture skewed inside a
  straight film strip comes out level.
- Large selections run in turns of 6 photos; running it again continues.
- Says which permission to allow when one is missing.
- **Split Half-Frame Scans…**: in a 35 mm gate scan of half-frame film, crops the original to one
  exposure and makes a virtual copy cropped to the other, both leveled with the gate. Finds the
  strip between the exposures whatever its tone (black, or film base on a raw negative), splits
  evenly when they touch, and takes 3 or 4 exposures per scan too. Running it again re-crops the
  copies it made.

Requires a Chromaforge build with the host features `pixels`, `crop` and `virtualCopy`.
