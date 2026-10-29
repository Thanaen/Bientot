"""Cuts the site's IBM Plex fonts down to what the pages use, in two files per face, and points the
pages at them.

Reads the usage report of browse.ts (the characters each face draws on each page) and writes, for
each face:
- a core file: printable ASCII, the no-break space, curly quotes, guillemets, en and em dashes and
  the ellipsis, plus every character the face's family draws on the site. The pages preload it.
- an extended file: the rest of Latin-1 and Latin Extended-A. Its unicode-range keeps browsers
  from downloading it until a page draws one of its characters.

Each file keeps only the OpenType features its text uses (kern; liga when the text has the letters
of a ligature) and the fonts' TrueType hinting (see README.md, Fonts). Plex is a Reserved Font Name
under the OFL, so the subsets are named Bientot Sans and Bientot Mono inside the files.

Then it rewrites the @font-face rules and font preloads of the pages, and src/fonts.ts.
Usage: python3 scripts/fonts/subset.py <usage report.json> [--no-hinting]
"""

import hashlib
import io
import json
import re
import sys
from pathlib import Path

from fontTools import subset
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.ttLib import TTFont

SERVICE = Path(__file__).resolve().parents[2]
SOURCES = SERVICE.parents[1] / "crates/bientot-app/assets/fonts"
OUT = SERVICE / "site/fonts"
# The files whose @font-face rules this script owns, the rendered pages (pages.ts) that show each
# one, and whether it preloads its fonts (the OG image's source is not served).
TEMPLATES = {
    "site/index.html": (r"index(-.*)?\.html", True),
    "site/statistiques.html": (r"statistiques-.*\.html", True),
    "site/og/home.html": (r"og/home\.html", False),
}

FACES = [
    # family, weight, source file, file name stem, name inside the subset
    ("IBM Plex Sans", 400, "IBMPlexSans-Regular.ttf", "sans-400"),
    ("IBM Plex Sans", 500, "IBMPlexSans-Medium.ttf", "sans-500"),
    ("IBM Plex Sans", 600, "IBMPlexSans-SemiBold.ttf", "sans-600"),
    ("IBM Plex Mono", 400, "IBMPlexMono-Regular.ttf", "mono-400"),
    ("IBM Plex Mono", 500, "IBMPlexMono-Medium.ttf", "mono-500"),
]
CORE = set(range(0x20, 0x7F)) | {0xA0, 0x2018, 0x2019, 0x201C, 0x201D, 0xAB, 0xBB, 0x2013, 0x2014, 0x2026}
EXTENDED = set(range(0xA0, 0x100)) | set(range(0x100, 0x180))
FEATURES = ["kern", "liga"]


def face_for(family: str, weight: int) -> tuple | None:
    """The face CSS font matching picks for a weight among the family's faces."""
    faces = [f for f in FACES if f[0] == family]
    if not faces:
        return None
    weights = sorted(f[1] for f in faces)
    if weight in weights:
        pick = weight
    elif 400 <= weight <= 500:
        up = [w for w in weights if weight < w <= 500]
        down = [w for w in weights if w < weight]
        pick = up[0] if up else (down[-1] if down else min(w for w in weights if w > 500))
    elif weight < 400:
        down = [w for w in weights if w < weight]
        pick = down[-1] if down else min(weights)
    else:
        up = [w for w in weights if w > weight]
        pick = up[0] if up else max(weights)
    return next(f for f in faces if f[1] == pick)


def usage(report: dict) -> tuple[dict, dict]:
    """The text each face draws on the site, and the faces each template's pages use."""
    text: dict[tuple, str] = {f: "" for f in FACES}
    used: dict[str, set] = {t: set() for t in TEMPLATES}
    for page, entry in report.items():
        template = next((t for t, (pages, _) in TEMPLATES.items() if re.fullmatch(pages, page)), None)
        if template is None:
            continue  # specimen.html is not a page of the site
        for key, lines in entry["faces"].items():
            stack, weight, style = key.split("|")
            family = stack.split(",")[0].strip().strip('"')
            face = face_for(family, int(weight))
            if face is None:
                continue
            text[face] += "\n" + lines
            used[template].add(face)
    return text, used


def ligature_inputs(font: TTFont, feature: str) -> list[str]:
    """The character sequences that a GSUB ligature feature replaces."""
    reverse = {g: chr(u) for u, g in font.getBestCmap().items()}
    gsub = font["GSUB"].table
    sequences = []
    for record in gsub.FeatureList.FeatureRecord:
        if record.FeatureTag != feature:
            continue
        for index in record.Feature.LookupListIndex:
            for table in gsub.LookupList.Lookup[index].SubTable:
                table = getattr(table, "ExtSubTable", table)
                for first, ligatures in getattr(table, "ligatures", {}).items():
                    for ligature in ligatures:
                        glyphs = [first, *ligature.Component]
                        if all(g in reverse for g in glyphs):
                            sequences.append("".join(reverse[g] for g in glyphs))
    return sequences


def features_for(font: TTFont, text: str) -> list[str]:
    tags = {r.FeatureTag for t in ("GSUB", "GPOS") if t in font for r in font[t].table.FeatureList.FeatureRecord}
    keep = []
    for feature in FEATURES:
        if feature not in tags:
            continue
        if feature == "liga" and not any(set(seq) <= set(text) for seq in ligature_inputs(font, "liga")):
            continue
        keep.append(feature)
    return keep


def rename(font: TTFont) -> None:
    """Replaces the Reserved Font Name in the names that identify the font."""
    for record in font["name"].names:
        if record.nameID in (1, 3, 4, 6, 16, 17, 18, 21, 22):
            value = record.toUnicode()
            record.string = value.replace("IBM Plex ", "Bientot ").replace("IBMPlex", "Bientot").replace("Plex", "Bientot")
    for record in font["name"].names:
        if record.nameID in (1, 3, 4, 6) and "Plex" in record.toUnicode():
            raise SystemExit(f"name {record.nameID} still says Plex: {record.toUnicode()}")


def cut(source: Path, unicodes: set, features: list[str], hinting: bool) -> bytes:
    options = subset.Options()
    options.flavor = "woff2"
    options.layout_features = features
    options.hinting = hinting
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6]
    options.notdef_outline = False
    # fontTools adds the mirror image of brackets and guillemets for right-to-left text, which the
    # site has none of; the unicode-range must list exactly what each file holds.
    options.bidi_closure = False
    font = subset.load_font(str(source), options)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=sorted(unicodes))
    subsetter.subset(font)
    rename(font)
    buffer = io.BytesIO()
    subset.save_font(font, buffer, options)
    return buffer.getvalue()


def check(data: bytes, source: Path, unicodes: set, features: list[str], hinting: bool) -> None:
    """Checks a subset against its source: same characters as asked, same outlines, advances,
    hinting and vertical metrics, only the features asked, and no Reserved Font Name."""
    font, original = TTFont(io.BytesIO(data)), TTFont(source)
    problems = []
    if set(font.getBestCmap()) != unicodes:
        problems.append(f"holds {len(font.getBestCmap())} characters, {len(unicodes)} asked")
    for table, fields in {
        "head": ["unitsPerEm"],
        "hhea": ["ascent", "descent", "lineGap"],
        "OS/2": ["sTypoAscender", "sTypoDescender", "sTypoLineGap", "usWinAscent", "usWinDescent", "fsSelection", "xAvgCharWidth"],
    }.items():
        for field in fields:
            if getattr(font[table], field) != getattr(original[table], field):
                problems.append(f"{table}.{field} changed")
    if font["gasp"].gaspRange != original["gasp"].gaspRange:
        problems.append("gasp changed")
    glyphs, originals = font.getGlyphSet(), original.getGlyphSet()
    cmap, source_cmap = font.getBestCmap(), original.getBestCmap()
    for u, name in cmap.items():
        a, b = DecomposingRecordingPen(glyphs), DecomposingRecordingPen(originals)
        glyphs[name].draw(a)
        originals[source_cmap[u]].draw(b)
        if a.value != b.value or glyphs[name].width != originals[source_cmap[u]].width:
            problems.append(f"U+{u:04X} drawn differently")
        program = lambda f, g: f["glyf"][g].program.getBytecode() if hasattr(f["glyf"][g], "program") else b""
        if hinting and program(font, name) != program(original, source_cmap[u]):
            problems.append(f"U+{u:04X} hinted differently")
    for table in ("fpgm", "prep", "cvt "):
        if hinting and (table not in font or font[table].compile(font) != original[table].compile(original)):
            problems.append(f"{table} changed")
        if not hinting and table in font:
            problems.append(f"{table} kept")
    tags = {r.FeatureTag for t in ("GSUB", "GPOS") if t in font for r in font[t].table.FeatureList.FeatureRecord}
    if not tags <= set(features):
        problems.append(f"features {sorted(tags)}")
    names = [r.toUnicode() for r in font["name"].names if r.nameID in (1, 3, 4, 6, 16, 17)]
    if any("Plex" in n for n in names):
        problems.append(f"named {names}")
    if problems:
        raise SystemExit(f"{source.name}: " + "; ".join(problems))


def ranges(codepoints: set) -> str:
    out, run = [], []
    for c in sorted(codepoints):
        if run and c == run[-1] + 1:
            run.append(c)
            continue
        if run:
            out.append(run)
        run = [c]
    if run:
        out.append(run)
    return ", ".join(f"U+{r[0]:X}" if len(r) == 1 else f"U+{r[0]:X}-{r[-1]:X}" for r in out)


def main() -> None:
    report = json.loads(Path(sys.argv[1]).read_text())
    hinting = "--no-hinting" not in sys.argv
    text, used = usage(report)

    families: dict[str, set] = {}
    for face, t in text.items():
        families.setdefault(face[0], set()).update(ord(c) for c in t if c != "\n")

    files = {}  # face -> [(path, unicode-range)]
    written = {}  # path -> bytes, written once every file passes its check
    for face in FACES:
        family, weight, source, stem = face
        source = SOURCES / source
        cmap = set(TTFont(source).getBestCmap())
        drawn = families[family]
        missing = {c for c in drawn if c not in cmap and c > 0x20}
        if missing:
            print(f"{stem}: drawn with a fallback font, as before: {''.join(map(chr, sorted(missing)))}")
        core = (CORE | drawn) & cmap
        extended = ((EXTENDED | drawn) & cmap) - core
        features = features_for(TTFont(source), text[face])
        files[face] = []
        for part, unicodes in (("core", core), ("ext", extended)):
            data = cut(source, unicodes, features, hinting)
            name = f"{stem}-{part}.{hashlib.sha256(data).hexdigest()[:8]}.woff2"
            written[name] = data
            check(data, source, unicodes, features, hinting)
            files[face].append((name, ranges(unicodes)))
            print(f"{name}: {len(data)} B, {len(unicodes)} characters, features {features or 'none'}")

    for old in OUT.glob("*.woff2"):
        old.unlink()
    for name, data in written.items():
        (OUT / name).write_bytes(data)

    rule = re.compile(r'@font-face \{ font-family: "(?P<family>[^"]+)"; font-weight: (?P<weight>\d+);(?P<display> font-display: \w+;)? src: url\("(?P<prefix>[^"]*?)fonts/[^"]+"\) format\("woff2"\);(?: unicode-range: [^;]+;)? \}\n')
    preload = re.compile(r'<link rel="preload" href="/fonts/[^"]+" as="font" type="font/woff2" crossorigin>\n')
    for template in TEMPLATES:
        path = SERVICE / template
        html = path.read_text()
        seen = []
        def replace(m: re.Match) -> str:
            face = next(f for f in FACES if f[0] == m["family"] and f[1] == int(m["weight"]))
            if face in seen:
                return ""
            seen.append(face)
            return "".join(
                f'@font-face {{ font-family: "{m["family"]}"; font-weight: {m["weight"]};{m["display"] or ""} src: url("{m["prefix"]}fonts/{name}") format("woff2"); unicode-range: {r}; }}\n'
                for name, r in files[face]
            )
        html = rule.sub(replace, html)
        if TEMPLATES[template][1]:
            links = "".join(
                f'<link rel="preload" href="/fonts/{files[f][0][0]}" as="font" type="font/woff2" crossorigin>\n'
                for f in FACES
                if f in used[template]
            )
            first = preload.search(html)
            if not first:
                raise SystemExit(f"{template}: no font preload to replace")
            html = html[: first.start()] + links + preload.sub("", html[first.start():])
        path.write_text(html)

    entries = "\n".join(
        f'  "/fonts/{name}": new URL("../site/fonts/{name}", import.meta.url),' for face in FACES for name, _ in files[face]
    )
    (SERVICE / "src/fonts.ts").write_text(
        "// Written by scripts/fonts/subset.py: the font files the pages use, named by their content, so that\n"
        "// browsers may keep them for good. Each one is named with a literal URL for Vercel's file tracing.\n"
        f"export const FONTS: Record<string, URL> = {{\n{entries}\n}};\n"
    )


if __name__ == "__main__":
    main()
