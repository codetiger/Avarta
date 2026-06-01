# Shell-shape comparison harness

A test setup that checks how well the generator reproduces **real shell species**.
For each species it gets the mesh out of the Rust core *through the existing wasm*
(the same `generate(params)` the web page calls), builds an OBJ in Python, renders
it, and pairs the render with a real reference photo in `report.html` for **human
review** — the script renders, you judge what doesn't match.

The Rust crates (`shell-core`, `shell-wasm`) are **not modified or rebuilt** by any
of this.

## Pipeline

```
species.json ─▶ render_catalog.py
                   │  per species:
                   │   params ─▶ node extract_mesh.mjs ─▶ (uses ../web/pkg wasm) ─▶ mesh arrays
                   │   arrays ─▶ build OBJ (Python) ─▶ pyvista offscreen render ─▶ rendered/<slug>.png
                   ▼
reference/<slug>.jpg ─────────▶ report.html   (real photo | generated render, per species)
```

- `extract_mesh.mjs` — Node bridge: loads the prebuilt `../web/pkg/shell_wasm.js`,
  calls `generate(params)` exactly like the browser, and writes the mesh
  (positions/normals/uvs/indices) as a binary stream to stdout.
- `render_catalog.py` — runs the bridge per species, builds the OBJ, renders it
  from a canonical pose (spire up; planispirals face-on), writes `report.html`.
- `species.json` — the catalog: name, family, generator `params`, a `coverage`
  rating (`good` / `silhouette` / `blocked`) and `notes` (what's missing).
- `fetch_references.py` — downloads each species' reference photo from Wikipedia /
  Wikimedia Commons and records the source in `species.json`.

## Requirements

- **Node.js** on PATH (to run the wasm the same way the web page does).
- **Python 3.9+** with the deps in `requirements.txt` (`pyvista`, `numpy`).
- A built `../web/pkg` (it's already in the repo; rebuild only if the Rust core
  changes, via `wasm-pack build crates/shell-wasm --target web --out-dir ../../web/pkg`).

## Setup & run

```bash
cd harness
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
python fetch_references.py      # download the reference photos (once)
python render_catalog.py        # render all species -> report.html
open report.html
```

Useful flags:

```bash
python render_catalog.py --only conus turritella   # subset (substring match on slug)
python render_catalog.py --size 720                # larger renders
python render_catalog.py --keep-obj                # keep the OBJ files in meshes/ (large)
```

## Adding a species

1. Append an entry to `species.json` (`slug`, `scientific`, `common`, `family`,
   `params`, `coverage`, `notes`, `reference`). `params` only needs the fields that
   differ from the generator defaults.
2. Add a reference photo at `reference/<slug>.jpg` (or add a Wikipedia title to
   `TITLE_OVERRIDES` in `fetch_references.py` and re-run it).
3. `python render_catalog.py --only <slug>` and check it in `report.html`.

No code changes are needed — the scripts loop over `species.json`.

## Coverage = backlog

`coverage` doubles as a coverage map. `silhouette` / `blocked` entries name the
missing generator feature in `notes` (e.g. cones, *Syrinx*, olives all want a
**siphonal canal + asymmetric aperture**). Filtering those is the feature backlog
for the generator.
