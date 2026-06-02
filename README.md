# Avarta

*Avarta* (Sanskrit आवर्त, "whorl / coil") is a mathematical generator for spiral
seashell shapes (snails, *Nautilus*, augers …).
Mesh math lives in a small Rust crate compiled to WebAssembly; a Vite-bundled web
app renders it with Three.js — a real-time **PBR++** viewport (image-based
lighting, physical material with translucency/iridescence/clearcoat, ambient
occlusion, ACES tone-mapping, bloom) plus an on-demand **path-traced "Render"**
mode (three-gpu-pathtracer) for photo-quality stills.

See [`parameters.md`](./parameters.md) for the parameter model and
[`scope.md`](./scope.md) for what shapes are in / out of scope.

## Layout

```
crates/avarta-core   Pure Rust mesh math (Raup W/D/T helico-spiral + ornament + seeded jitter).
crates/avarta-wasm   wasm-bindgen adapter -> JS typed arrays (positions/normals/uvs/indices).
web/                Vite app: index.html + <avarta-viewer> component (+ generated pkg/).
.github/workflows   GitHub Pages build & deploy (wasm + Vite).
```

## Prerequisites

- Rust (stable) + the wasm target: `rustup target add wasm32-unknown-unknown`
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/)
- Node.js 18+ (for Vite)

## Develop

```sh
# 1. test the pure math natively
cargo test -p avarta-core

# 2. build the wasm package into web/pkg/
wasm-pack build crates/avarta-wasm --target web --out-dir ../../web/pkg

# 3. run the Vite dev server (installs deps on first run)
cd web && npm install && npm run dev
# open http://localhost:8090
```

After changing Rust, re-run `wasm-pack` (step 2); Vite hot-reloads the rest.
Vite bundles `three` + `three-gpu-pathtracer` from npm (one deduped `three`
instance — required for the path tracer) and treats the `.wasm` as an asset.

## Deploy (GitHub Pages)

1. Push to a GitHub repo (`git init && git add -A && git commit && git push`).
2. Repo **Settings → Pages → Build and deployment → Source: GitHub Actions**.
3. Pushing to `main` runs `.github/workflows/deploy.yml`: native tests → build
   wasm → `npm ci && npm run build` (Vite) → publish `web/dist/`. The Vite base
   is `./` so it works under the `/<repo>/` Pages subpath.

## Embed elsewhere

The `<avarta-viewer>` element renders into its own shadow DOM. Because the app is
now bundled, embed it by pointing at the built component bundle from `web/dist`
(rather than the raw source). Attributes drive the shape, e.g.
`<avarta-viewer w="2.0" d="0.15" t="1.5" n="5" aspect="1.0">`.
