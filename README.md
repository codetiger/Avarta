# Spiral

A mathematical generator for spiral shell shapes (snails, *Nautilus*, augers …).
Mesh math lives in a small Rust crate compiled to WebAssembly; a no-bundler web
page renders it with Three.js. Hosted on GitHub Pages and embeddable as a
`<shell-viewer>` web component in any page (e.g. an Astro blog).

See [`parameters.md`](./parameters.md) for the parameter model and
[`scope.md`](./scope.md) for what shapes are in / out of scope.

## Layout

```
crates/shell-core   Pure Rust mesh math (Raup W/D/T helico-spiral). cargo-testable.
crates/shell-wasm   wasm-bindgen adapter -> JS typed arrays.
web/                Static no-bundler page: index.html + shell-viewer.js (+ pkg/).
.github/workflows   GitHub Pages build & deploy.
```

## Prerequisites

- Rust (stable) + the wasm target: `rustup target add wasm32-unknown-unknown`
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/)

## Develop

```sh
# 1. test the pure math natively
cargo test -p shell-core

# 2. build the wasm package into web/pkg/
wasm-pack build crates/shell-wasm --target web --out-dir ../../web/pkg

# 3. serve the static page with no-cache headers
python3 dev-server.py 8080
# open http://localhost:8080
```

> Use `dev-server.py`, not `python3 -m http.server`: browsers cache the `.wasm`
> aggressively, so after a rebuild a plain server can keep running the OLD wasm
> (which silently ignores newly added parameters). `dev-server.py` sends
> `Cache-Control: no-store`. After a rebuild, hard-reload once (⌘⇧R) to be safe.

Three.js is loaded from esm.sh and the `.wasm` is resolved relative to the
module, so the page is fully static — no bundler, no install step.

## Deploy (GitHub Pages)

1. Push to a GitHub repo (`git init && git add -A && git commit && git push`).
2. Repo **Settings → Pages → Build and deployment → Source: GitHub Actions**.
3. Pushing to `main` runs `.github/workflows/deploy.yml`, which builds the wasm
   and publishes `web/`.

## Embed in Astro (or any page)

The component is self-contained — drop two lines into an `.mdx` post:

```html
<script type="module" src="https://<user>.github.io/spiral/shell-viewer.js"></script>
<shell-viewer w="2.0" d="0.15" t="1.5" n="5" aspect="1.0"
              style="display:block;height:420px"></shell-viewer>
```

It lazy-loads the wasm + Three.js and renders into its own shadow DOM, so it
won't collide with the host page's styles.
