"""Writes ChromaGate's logo and icon into brand/ (original work, MIT).

The wordmark: "Chroma" in the ChromaLab spectrum, "at" in grey, and a squared G and e in the accent
colour whose corners are the top-left and bottom-right corners of a crop box. The icon: the squared
G in the spectrum with the accent's crop corner at bottom-right, on a rounded plate (none, dark,
light), as ChromaLab's.

    python tools/brand.py
"""
from pathlib import Path

BRAND = Path(__file__).resolve().parent.parent / "brand"

# ChromaLab's palette
SPECTRUM = [(0, "#ff4d6d"), (0.3, "#ff9a3c"), (0.55, "#ffd23f"), (0.78, "#3bceac"), (1, "#4d8dff")]
THEMES = {
    # wordmark grey, accent
    "light": ("#2a2d34", "#e8901a"),
    "dark": ("#e8eaee", "#ffb03a"),
}
PLATES = {"": "none", "-dark": "#1b1d22", "-light": "#f4f5f7"}


def gradient(x1, y1, x2, y2):
    stops = "".join(f'<stop offset="{o}" stop-color="{c}"/>' for o, c in SPECTRUM)
    return (f'<linearGradient id="spectrum" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" '
            f'x2="{x2}" y2="{y2}">{stops}</linearGradient>')


def wordmark(grey, accent):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2240 500" width="2240" height="500">
<title>ChromaGate</title>
<defs>{gradient(50, 0, 1300, 0)}</defs>
<g fill="none" stroke-width="24" stroke-linecap="round" stroke-linejoin="round">
<g stroke="url(#spectrum)">
<path d="M 272 184 A 122 122 0 1 0 272 349"/>
<path d="M 335 104 L 335 388"/><path d="M 335 320 A 67 67 0 0 1 469 320 L 469 388"/>
<path d="M 533 388 L 533 320 A 67 67 0 0 1 600 253 L 617 253"/>
<circle cx="747" cy="320" r="68"/>
<path d="M 877 388 L 877 304 A 53 53 0 0 1 983 304 L 983 388"/><path d="M 983 304 A 53 53 0 0 1 1089 304 L 1089 388"/>
<circle cx="1210" cy="320" r="68"/><path d="M 1287 253 L 1287 388"/>
</g>
<path stroke="{accent}" d="M 1600 148 L 1380 148 L 1380 330 A 58 58 0 0 0 1438 388 L 1542 388 A 58 58 0 0 0 1600 330 L 1600 280 L 1500 280"/>
<g stroke="{grey}">
<circle cx="1730" cy="320" r="68"/><path d="M 1807 253 L 1807 388"/>
<path d="M 1890 160 L 1890 350 A 38 38 0 0 0 1928 388 L 1940 388"/><path d="M 1855 253 L 1935 253"/>
</g>
<path stroke="{accent}" d="M 1990 320 L 2130 320 A 70 70 0 1 0 2060 390 L 2186 390 L 2186 300"/>
</g>
</svg>
'''


def icon(plate, accent):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="512" height="512">
<title>ChromaGate</title>
<defs>{gradient(66, 196, 186, 76)}</defs>
<rect x="8" y="8" width="240" height="240" rx="52" fill="{plate}"/>
<g fill="none" stroke-width="16" stroke-linecap="round" stroke-linejoin="round">
<path stroke="url(#spectrum)" d="M 176 62 L 62 62 L 62 150 A 34 34 0 0 0 96 184 L 142 184 A 34 34 0 0 0 176 150 L 176 124 L 124 124"/>
<path stroke="{accent}" d="M 152 210 L 210 210 L 210 152"/>
</g>
</svg>
'''


def main():
    BRAND.mkdir(exist_ok=True)
    for theme, (grey, accent) in THEMES.items():
        (BRAND / f"logo-{theme}.svg").write_text(wordmark(grey, accent), encoding="utf-8", newline="\n")
    for suffix, plate in PLATES.items():
        # the accent of the theme the plate is for (the plateless icon takes the dark theme's)
        accent = THEMES["light" if suffix == "-light" else "dark"][1]
        (BRAND / f"icon{suffix}.svg").write_text(icon(plate, accent), encoding="utf-8", newline="\n")
    for f in sorted(BRAND.glob("*.svg")):
        size = f.stat().st_size
        assert size <= 24 * 1024, f"{f.name}: {size} bytes, over the host's 24 KiB"
        print(f"{f.name}: {size} bytes")


if __name__ == "__main__":
    main()
