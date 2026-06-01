#!/usr/bin/env python3
"""Shell-shape comparison harness.

For each species in ``species.json``:
  1. get the mesh + Layer-3 pigment field from the Rust core *through the
     existing wasm* — the same ``generate(params)`` the web page calls — via the
     Node bridge ``extract_mesh.mjs`` (no Rust code is touched here);
  2. map the pigment field through the species' palette (PIGMENTS) into a
     texture and apply it via the mesh's UVs (the same LUT the web viewer bakes);
  3. render it offscreen with pyvista from a canonical "spire-up" pose
     (build a Wavefront OBJ too with --keep-obj, for external inspection);
  4. write ``report.html`` pairing each pigmented render with the real reference
     photo, so a human can scan for shapes/patterns that don't match.

Usage:
    python render_catalog.py                 # all species
    python render_catalog.py --only conus-marmoreus auger ...
    python render_catalog.py --size 640
"""
from __future__ import annotations

import argparse
import json
import os
import struct
import subprocess
import sys
import tempfile
from html import escape
from pathlib import Path

import numpy as np
import pyvista as pv

HERE = Path(__file__).resolve().parent
BRIDGE = HERE / "extract_mesh.mjs"
SPECIES_JSON = HERE / "species.json"
REF_DIR = HERE / "reference"
MESH_DIR = HERE / "meshes"
OUT_DIR = HERE / "rendered"
REPORT = HERE / "report.html"

SHELL_COLOR = "#e7d8b6"  # warm cream, neutral so shape (not colour) is judged
COVERAGE_COLOR = {"good": "#3fa45b", "silhouette": "#c8a35a", "blocked": "#c0524a"}

# Per-species Layer-3 pigmentation (RD `pig_*`) + Layer-4 `palette`, keyed by
# slug — mirrors the web viewer's SPECIES table (web/index.html) and adds the
# extra harness-only species (Oliva/Conus/Cypraea …). Regime indices match
# PigRegime in shell-core: 0 solid · 1 spiral bands · 2 axial stripes ·
# 3 oblique lines · 4 chevrons/tented · 5 spots · 6 reticulated. Unlisted slugs
# fall back to DEFAULT_PIG.
REGIME_NAMES = ["solid", "spiral bands", "axial stripes", "oblique lines",
                "chevrons / tented", "spots", "reticulated"]
DEFAULT_PIG = {"pig": {"pig_regime": 0}, "palette": ("#e7d8b6", "#cdb98a", "#a07a4a")}
PIGMENTS = {
    "nautilus-pompilius":        {"pig": {"pig_regime": 2, "pig_scale": 0.45, "pig_contrast": 0.7, "pig_density": 0.6, "pig_irregularity": 0.25}, "palette": ("#f4ead2", "#c8763f", "#9c3f1e")},
    "planorbis-planorbis":       {"pig": {"pig_regime": 0}, "palette": ("#7a5230", "#5e3d22", "#3f2814")},
    "spirula-spirula":           {"pig": {"pig_regime": 0}, "palette": ("#f3efe4", "#e4dccb", "#cdc4ad")},
    "architectonica-perspectiva":{"pig": {"pig_regime": 1, "pig_scale": 0.35, "pig_contrast": 0.8, "pig_density": 0.5, "pig_irregularity": 0.1}, "palette": ("#dac49a", "#9a6a38", "#5a3415")},
    "helix-pomatia":             {"pig": {"pig_regime": 1, "pig_scale": 0.6, "pig_contrast": 0.35, "pig_density": 0.25, "pig_irregularity": 0.2}, "palette": ("#d8b98a", "#a87b4e", "#7a542f")},
    "cepaea-nemoralis":          {"pig": {"pig_regime": 1, "pig_scale": 0.55, "pig_contrast": 0.85, "pig_density": 0.3, "pig_irregularity": 0.05}, "palette": ("#e7c64a", "#9c6b2a", "#3f2611")},
    "littorina-littorea":        {"pig": {"pig_regime": 1, "pig_scale": 0.2, "pig_contrast": 0.6, "pig_density": 0.3, "pig_irregularity": 0.15}, "palette": ("#9d8d74", "#5f5142", "#332b22")},
    "trochus-niloticus":         {"pig": {"pig_regime": 3, "pig_scale": 0.4, "pig_contrast": 0.75, "pig_density": 0.55, "pig_angle": 0.6, "pig_irregularity": 0.2}, "palette": ("#eadfc8", "#b65a3a", "#7e2f1c")},
    "turritella-terebra":        {"pig": {"pig_regime": 1, "pig_scale": 0.18, "pig_contrast": 0.65, "pig_density": 0.3, "pig_irregularity": 0.15}, "palette": ("#cdb38a", "#9a6c40", "#6a3f20")},
    "cerithium-nodulosum":       {"pig": {"pig_regime": 1, "pig_scale": 0.35, "pig_contrast": 0.75, "pig_density": 0.4, "pig_irregularity": 0.2}, "palette": ("#3a2c1c", "#8a6e48", "#e8dcc0")},
    "cancellaria-reticulata":    {"pig": {"pig_regime": 6, "pig_scale": 0.5, "pig_contrast": 0.7, "pig_density": 0.55, "pig_irregularity": 0.2}, "palette": ("#e6cda2", "#b07a3f", "#7c3f1a")},
    "bursa-bufonia":             {"pig": {"pig_regime": 6, "pig_scale": 0.65, "pig_contrast": 0.55, "pig_density": 0.45, "pig_irregularity": 0.5}, "palette": ("#d6c09a", "#9a6a40", "#5c3a1f")},
    "terebra-maculata":          {"pig": {"pig_regime": 5, "pig_scale": 0.5, "pig_contrast": 0.8, "pig_density": 0.6, "pig_irregularity": 0.2}, "palette": ("#f0e7d2", "#caa05a", "#9c5a24")},
    # Harness-only species (not in the web list):
    "oliva-porphyria":           {"pig": {"pig_regime": 4, "pig_scale": 0.4, "pig_contrast": 0.8, "pig_density": 0.5, "pig_angle": 0.6, "pig_irregularity": 0.3}, "palette": ("#e7d3ad", "#9c6a3c", "#4a2e18")},
    "conus-marmoreus":           {"pig": {"pig_regime": 4, "pig_scale": 0.45, "pig_contrast": 0.9, "pig_density": 0.55, "pig_angle": 0.55, "pig_irregularity": 0.35}, "palette": ("#17130f", "#7a7468", "#efe9da")},
    "cypraea-tigris":            {"pig": {"pig_regime": 5, "pig_scale": 0.5, "pig_contrast": 0.85, "pig_density": 0.55, "pig_irregularity": 0.4}, "palette": ("#d8cbb0", "#6a5f4a", "#2a241c")},
    "murex-pecten":              {"pig": {"pig_regime": 0}, "palette": ("#e8e0d0", "#d2c6ac", "#b3a07e")},
    "syrinx-aruanus":            {"pig": {"pig_regime": 0}, "palette": ("#e8c79a", "#d2ac79", "#b3895a")},
    "bullata-bullata":           {"pig": {"pig_regime": 1, "pig_scale": 0.5, "pig_contrast": 0.3, "pig_density": 0.2, "pig_irregularity": 0.2}, "palette": ("#d8b48a", "#bb9266", "#9a6f44")},
    "lambis-lambis":             {"pig": {"pig_regime": 6, "pig_scale": 0.7, "pig_contrast": 0.5, "pig_density": 0.4, "pig_irregularity": 0.5}, "palette": ("#d8c4a0", "#9a6e44", "#6a4a2a")},
}


def _hex_rgb(h: str) -> np.ndarray:
    h = h.lstrip("#")
    return np.array([int(h[i:i + 2], 16) for i in (0, 2, 4)], np.float32)


def pigment_texture(field: np.ndarray, palette: tuple[str, str, str]) -> np.ndarray:
    """Map the 0..255 pigment field through a 3-stop palette (base → accent →
    pattern) into an RGB image, the same LUT the web viewer bakes."""
    base, accent, pattern = (_hex_rgb(c) for c in palette)
    lut = np.empty((256, 3), np.float32)
    half = np.arange(128) / 128.0
    lut[:128] = base + (accent - base) * half[:, None]
    lut[128:] = accent + (pattern - accent) * (np.arange(128) / 128.0)[:, None]
    return lut[field].astype(np.uint8)  # (H, W, 3)


# ----------------------------------------------------------------------------- mesh
def get_mesh(params: dict):
    """Run the Node bridge and decode its binary mesh stream →
    (pos, nor, uvs, idx, pigment[H,W], pig_w, pig_h)."""
    proc = subprocess.run(
        ["node", str(BRIDGE), json.dumps(params)], capture_output=True
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.decode() or "extract_mesh.mjs failed")
    b = proc.stdout
    n_pos, n_nor, n_uv, n_idx, n_pig, pig_w, pig_h = struct.unpack_from("<7I", b, 0)
    off = 28
    pos = np.frombuffer(b, np.float32, n_pos, off).reshape(-1, 3); off += n_pos * 4
    nor = np.frombuffer(b, np.float32, n_nor, off).reshape(-1, 3); off += n_nor * 4
    uvs = np.frombuffer(b, np.float32, n_uv, off).reshape(-1, 2); off += n_uv * 4
    idx = np.frombuffer(b, np.uint32, n_idx, off).reshape(-1, 3); off += n_idx * 4
    pig = np.frombuffer(b, np.uint8, n_pig, off).reshape(pig_h, pig_w)
    return pos, nor, uvs, idx, pig, pig_w, pig_h


def write_obj(path: Path, pos: np.ndarray, nor: np.ndarray, idx: np.ndarray) -> None:
    """Generate a Wavefront OBJ (the OBJ is built here in Python, not in Rust)."""
    chunks = ["# generated by harness/render_catalog.py\n"]
    chunks.append("\n".join(f"v {x:.6g} {y:.6g} {z:.6g}" for x, y, z in pos))
    chunks.append("\n" + "\n".join(f"vn {x:.5g} {y:.5g} {z:.5g}" for x, y, z in nor))
    f = idx + 1  # OBJ is 1-indexed
    chunks.append("\n" + "\n".join(f"f {a}//{a} {b}//{b} {c}//{c}" for a, b, c in f))
    path.write_text("".join(chunks))


# --------------------------------------------------------------------------- render
def render(pos, nor, uvs, idx, tex_img, png_path: Path, size: int, t: float) -> None:
    """Render the mesh offscreen in a canonical pose, pigment texture applied.

    The generator orients the coil axis to +Y, the pointed apex to -Y, and the
    body whorl/aperture toward +Z. The model is a *hollow* swept tube (no
    columella), so looking into the aperture (+Z) shows the cavity. We therefore
    view spired shells from the dorsal (abapertural, -Z) side at a 3/4 angle with
    the spire up, which shows the solid convex form; planispirals (T~0) have no
    spire, so we look face-on down the coil axis to show the coil.
    """
    faces = np.empty((len(idx), 4), np.int64)
    faces[:, 0] = 3
    faces[:, 1:] = idx
    mesh = pv.PolyData(pos, faces.ravel())
    mesh.point_data["Normals"] = nor
    mesh.active_texture_coordinates = uvs.astype(np.float32)
    # Pigment field → texture; v (φ around the lip) repeats, u (θ) clamps.
    tex = pv.Texture(tex_img)
    tex.repeat = True

    pl = pv.Plotter(off_screen=True, window_size=[size, size])
    pl.set_background("white")
    pl.add_mesh(
        mesh, texture=tex, smooth_shading=True,
        ambient=0.25, diffuse=0.8, specular=0.25, specular_power=16,
    )
    center = np.asarray(mesh.center)
    pl.camera.focal_point = center
    if t < 0.15:  # planispiral — view the coil face-on, down the +Y axis
        pl.camera.up = (0.0, 0.0, -1.0)
        pl.camera.position = center + np.array([0.0, 1.0, 0.0])
    else:  # spired — dorsal 3/4 view, apex (-Y) up
        pl.camera.up = (0.0, -1.0, 0.0)
        pl.camera.position = center + np.array([0.4, 0.0, -1.0])
    pl.reset_camera()       # fit distance to bounds, keeping direction + up
    pl.camera.zoom(1.35)
    pl.screenshot(str(png_path))
    pl.close()


# --------------------------------------------------------------------------- report
def coverage_chip(cov: str) -> str:
    color = COVERAGE_COLOR.get(cov, "#777")
    return (f'<span class="chip" style="background:{color}">{escape(cov)}</span>')


def build_report(rows: list[dict]) -> None:
    counts = {"good": 0, "silhouette": 0, "blocked": 0}
    for r in rows:
        counts[r["coverage"]] = counts.get(r["coverage"], 0) + 1
    bar = " · ".join(
        f'<b style="color:{COVERAGE_COLOR[k]}">{v} {k}</b>' for k, v in counts.items()
    )

    cards = []
    for r in rows:
        ref = REF_DIR / Path(r["reference"]).name if r.get("reference") else None
        ref_html = (
            f'<img src="reference/{escape(ref.name)}" alt="reference">'
            if ref and ref.exists()
            else '<div class="missing">no reference photo<br>'
            f'<small>{escape(r.get("reference",""))}</small></div>'
        )
        render_html = (
            f'<img src="rendered/{escape(r["slug"])}.png" alt="render">'
            if r.get("ok")
            else f'<div class="missing err">render failed<br><small>{escape(r.get("error",""))}</small></div>'
        )
        gen_cap = "generated" + (f' · {escape(r["regime"])}' if r.get("regime") else "")
        src = r.get("reference_source", "")
        src_html = f'<a href="{escape(src)}" target="_blank">photo source</a>' if src else ""
        cards.append(f"""
      <div class="card">
        <div class="head">
          <span class="sci">{escape(r["scientific"])}</span>
          <span class="com">{escape(r.get("common",""))} · {escape(r.get("family",""))}</span>
          {coverage_chip(r["coverage"])}
        </div>
        <div class="pair">
          <figure>{ref_html}<figcaption>real shell {src_html}</figcaption></figure>
          <figure>{render_html}<figcaption>{gen_cap}</figcaption></figure>
        </div>
        <p class="notes">{escape(r.get("notes",""))}</p>
        <pre class="params">{escape(json.dumps(r["params"]))}</pre>
      </div>""")

    html = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<title>Shell catalog — generated vs. real</title>
<style>
  :root {{ color-scheme: light; }}
  body {{ margin:0; font:14px/1.5 system-ui,sans-serif; background:#f4f1ea; color:#23272e; }}
  header {{ padding:18px 24px; border-bottom:1px solid #e0dccf; background:#fbf9f3; position:sticky; top:0; }}
  h1 {{ margin:0 0 4px; font-size:18px; }}
  .sub {{ color:#777; font-size:12px; }}
  .grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(440px,1fr)); gap:18px; padding:24px; }}
  .card {{ border:1px solid #e0dccf; border-radius:10px; background:#fff; overflow:hidden; }}
  .head {{ display:flex; align-items:baseline; gap:8px; flex-wrap:wrap; padding:10px 14px; border-bottom:1px solid #efece2; }}
  .sci {{ font-weight:600; font-style:italic; }}
  .com {{ color:#888; font-size:12px; flex:1; }}
  .chip {{ color:#fff; font-size:10px; text-transform:uppercase; letter-spacing:.04em; padding:2px 7px; border-radius:10px; }}
  .pair {{ display:grid; grid-template-columns:1fr 1fr; }}
  figure {{ margin:0; }}
  figure img, .missing {{ width:100%; aspect-ratio:1; object-fit:cover; display:block; background:#faf8f2; }}
  .missing {{ display:flex; flex-direction:column; align-items:center; justify-content:center; color:#aaa; text-align:center; }}
  .missing.err {{ color:#c0524a; }}
  figcaption {{ font-size:11px; color:#999; text-align:center; padding:4px; }}
  .notes {{ margin:0; padding:10px 14px; font-size:12px; color:#555; }}
  .params {{ margin:0; padding:8px 14px 12px; font-size:11px; color:#999; white-space:pre-wrap; word-break:break-all; }}
  a {{ color:#b07a2a; }}
</style></head><body>
<header><h1>Shell catalog — generated vs. real</h1>
<div class="sub">{len(rows)} species · {bar} · meshes from the Rust core via web/pkg wasm</div></header>
<div class="grid">{''.join(cards)}</div>
</body></html>"""
    REPORT.write_text(html)


# ----------------------------------------------------------------------------- main
def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--only", nargs="*", help="render only these slugs (substring match)")
    ap.add_argument("--size", type=int, default=560, help="render size in px")
    ap.add_argument("--keep-obj", action="store_true",
                    help="persist each OBJ in meshes/ (large) instead of a temp file")
    args = ap.parse_args()

    pv.OFF_SCREEN = True
    OUT_DIR.mkdir(exist_ok=True)
    if args.keep_obj:
        MESH_DIR.mkdir(exist_ok=True)

    species = json.loads(SPECIES_JSON.read_text())
    if args.only:
        species = [s for s in species if any(k in s["slug"] for k in args.only)]

    rows = []
    for i, sp in enumerate(species, 1):
        slug = sp["slug"]
        print(f"[{i}/{len(species)}] {slug} … ", end="", flush=True)
        row = dict(sp, ok=False)
        try:
            pig_info = PIGMENTS.get(slug, DEFAULT_PIG)
            # Pass the shape params + the species' pigmentation params together —
            # the RD field is generated by the same generate() call as the mesh.
            params = {**sp["params"], **pig_info["pig"]}
            pos, nor, uvs, idx, pig, pig_w, pig_h = get_mesh(params)
            tex_img = pigment_texture(pig, pig_info["palette"])
            render(pos, nor, uvs, idx, tex_img, OUT_DIR / f"{slug}.png", args.size,
                   float(sp["params"].get("t", 1.5)))
            # The OBJ (geometry only) is large; keep it just for inspection.
            if args.keep_obj:
                write_obj(MESH_DIR / f"{slug}.obj", pos, nor, idx)
            row["ok"] = True
            row["regime"] = REGIME_NAMES[int(pig_info["pig"].get("pig_regime", 0))]
            print(f"{len(pos)} verts · {row['regime']} ✓")
        except Exception as e:  # keep going; the report shows the failure
            row["error"] = str(e)
            print(f"FAILED: {e}", file=sys.stderr)
        rows.append(row)

    build_report(rows)
    missing = [r["slug"] for r in rows if r.get("reference") and not (REF_DIR / Path(r["reference"]).name).exists()]
    if missing:
        print(f"\n{len(missing)} species missing reference photos: {', '.join(missing)}")
    print(f"\nReport: {REPORT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
