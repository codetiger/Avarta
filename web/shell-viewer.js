// <shell-viewer> — a self-contained custom element that renders a shell mesh.
//
// It loads the wasm generator and Three.js itself, so embedding it anywhere
// (the standalone page, or an Astro MDX post) is just:
//   <script type="module" src=".../shell-viewer.js"></script>
//   <shell-viewer w="2" d="0.15" t="1.5" n="5"></shell-viewer>
//
// Three.js is imported from esm.sh (which rewrites the addons' bare `three`
// imports), so no import map is needed on the host page.

import * as THREE from "https://esm.sh/three@0.160.0";
import { OrbitControls } from "https://esm.sh/three@0.160.0/examples/jsm/controls/OrbitControls.js";
import init, { generate } from "./pkg/shell_wasm.js";

// Initialise the wasm module once, shared across all <shell-viewer> instances.
// The .wasm URL is resolved relative to THIS module, so it works under a
// GitHub Pages subpath and when hot-linked from another origin.
let wasmReady;
function ensureWasm() {
  if (!wasmReady) {
    wasmReady = init(new URL("./pkg/shell_wasm_bg.wasm", import.meta.url));
  }
  return wasmReady;
}

const DEFAULTS = {
  w: 2.0, d: 0.15, t: 1.5, n: 5.0, aspect: 1.0,
  rib_ax_count: 0, rib_ax_amp: 0, rib_sp_count: 0, rib_sp_amp: 0, rib_sharp: 0,
  proj_count: 0, proj_rows: 0, proj_pos: 0, proj_size: 0, proj_sharp: 0,
  varix_count: 0, varix_amp: 0,
  seed: 0, jitter: 0,
  seg_theta: 96, seg_phi: 48,
};
const ATTRS = [
  "w", "d", "t", "n", "aspect",
  "rib_ax_count", "rib_ax_amp", "rib_sp_count", "rib_sp_amp", "rib_sharp",
  "proj_count", "proj_rows", "proj_pos", "proj_size", "proj_sharp",
  "varix_count", "varix_amp",
  "seed", "jitter",
];

class ShellViewer extends HTMLElement {
  static get observedAttributes() {
    return ATTRS;
  }

  constructor() {
    super();
    this.params = { ...DEFAULTS };
    this._loaded = false;
    this._framed = false;
    this.attachShadow({ mode: "open" });
  }

  connectedCallback() {
    const style = document.createElement("style");
    style.textContent = `:host{display:block;width:100%;height:100%;min-height:320px}
      canvas{display:block;width:100%;height:100%;touch-action:none}`;
    this.shadowRoot.append(style);

    this._initThree();

    // Pick up params declared as attributes (Astro embedding path).
    for (const a of ATTRS) {
      if (this.hasAttribute(a)) this.params[a] = parseFloat(this.getAttribute(a));
    }

    ensureWasm().then(() => {
      this._loaded = true;
      this._rebuild();
      this._animate();
    });

    this._ro = new ResizeObserver(() => this._onResize());
    this._ro.observe(this);
  }

  disconnectedCallback() {
    this._ro?.disconnect();
    cancelAnimationFrame(this._raf);
    this.renderer?.dispose();
  }

  attributeChangedCallback(name, _old, value) {
    if (value == null) return;
    this.params[name] = parseFloat(value);
    if (this._loaded) this._rebuild();
  }

  /** Imperative API used by the standalone page's form. */
  setParams(patch) {
    Object.assign(this.params, patch);
    if (this._loaded) this._rebuild();
  }

  _initThree() {
    const w = this.clientWidth || 600;
    const h = this.clientHeight || 400;

    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    this.renderer.setSize(w, h, false);
    this.shadowRoot.append(this.renderer.domElement);

    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color(0x0e1116);

    this.camera = new THREE.PerspectiveCamera(45, w / h, 0.01, 1000);
    this.camera.position.set(3, 2, 4);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;

    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x202830, 1.0));
    const key = new THREE.DirectionalLight(0xffffff, 1.6);
    key.position.set(5, 8, 5);
    this.scene.add(key);

    this.material = new THREE.MeshStandardMaterial({
      color: 0xe7d8b6,
      roughness: 0.5,
      metalness: 0.05,
      side: THREE.DoubleSide,
    });
    this.mesh = new THREE.Mesh(new THREE.BufferGeometry(), this.material);
    this.scene.add(this.mesh);
  }

  _rebuild() {
    let m;
    try {
      m = generate(this.params);
    } catch (e) {
      console.error("[shell-viewer] generate failed:", e);
      return;
    }
    const positions = m.positions;
    const normals = m.normals;
    const indices = m.indices;
    m.free(); // release the wasm-side struct; typed arrays above are JS-owned

    const geo = this.mesh.geometry;
    geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    geo.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    geo.setIndex(new THREE.BufferAttribute(indices, 1));
    geo.computeBoundingSphere();

    // Keep the shell centred at the origin so orbiting stays natural.
    const s = geo.boundingSphere;
    this.mesh.position.set(-s.center.x, -s.center.y, -s.center.z);

    // Frame the camera once on first build; afterwards leave the user's view.
    if (!this._framed) {
      const r = Math.max(s.radius, 1e-3);
      this.camera.position.setLength(r * 3);
      this.camera.near = r / 100;
      this.camera.far = r * 100;
      this.camera.updateProjectionMatrix();
      this.controls.target.set(0, 0, 0);
      this._framed = true;
    }
  }

  _onResize() {
    const w = this.clientWidth;
    const h = this.clientHeight;
    if (!w || !h) return;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  _animate() {
    this._raf = requestAnimationFrame(() => this._animate());
    this.controls.update();
    this.renderer.render(this.scene, this.camera);
  }
}

customElements.define("shell-viewer", ShellViewer);
export { ShellViewer };
