#!/usr/bin/env python3
"""Download a representative reference photo for each catalogued species.

Pulls each species' lead image from the Wikipedia REST summary API (images there
are Wikimedia Commons, CC / public-domain), saves it to ``reference/<slug>.jpg``,
and records the image + page URL back into ``species.json`` (``reference_source``).

Run once (or re-run to refill any that are missing):
    python fetch_references.py
"""
from __future__ import annotations

import json
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPECIES_JSON = HERE / "species.json"
REF_DIR = HERE / "reference"
UA = "avarta-harness/1.0 (local research; contact: nharishankar@gmail.com)"

# slug -> Wikipedia article title to pull the lead image from. Defaults to the
# scientific name; listed here only where that differs or needs disambiguation.
TITLE_OVERRIDES = {
    "spirula-spirula": "Spirula",
    "trochus-niloticus": "Tectus niloticus",
    "bullata-bullata": "Bullata bullata",
}


def get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read()


def lead_image(title: str) -> tuple[str, str] | None:
    """Return (image_url, page_url) for a Wikipedia article's lead image."""
    api = "https://en.wikipedia.org/api/rest_v1/page/summary/" + urllib.parse.quote(
        title.replace(" ", "_")
    )
    try:
        data = json.loads(get(api))
    except Exception as e:
        print(f"    summary API failed: {e}", file=sys.stderr)
        return None
    img = data.get("originalimage", {}).get("source") or data.get(
        "thumbnail", {}
    ).get("source")
    page = data.get("content_urls", {}).get("desktop", {}).get("page", "")
    return (img, page) if img else None


def main() -> int:
    REF_DIR.mkdir(exist_ok=True)
    species = json.loads(SPECIES_JSON.read_text())
    ok = 0
    for sp in species:
        slug = sp["slug"]
        dest = REF_DIR / f"{slug}.jpg"
        title = TITLE_OVERRIDES.get(slug, sp["scientific"])
        print(f"{slug}  ←  {title}")
        res = lead_image(title)
        if not res:
            print("    no lead image found", file=sys.stderr)
            continue
        img_url, page_url = res
        try:
            dest.write_bytes(get(img_url))
        except Exception as e:
            print(f"    download failed: {e}", file=sys.stderr)
            continue
        sp["reference_source"] = img_url
        sp["reference_page"] = page_url
        ok += 1
        print(f"    saved {dest.name} ({dest.stat().st_size // 1024} KB)")
        time.sleep(0.3)  # be polite to the API

    SPECIES_JSON.write_text(json.dumps(species, indent=2) + "\n")
    print(f"\n{ok}/{len(species)} reference photos downloaded.")
    missing = [s["slug"] for s in species if not (REF_DIR / f"{s['slug']}.jpg").exists()]
    if missing:
        print("Missing:", ", ".join(missing))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
