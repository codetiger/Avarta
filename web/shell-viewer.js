// <shell-viewer> — a custom element that renders a shell mesh.
//
// Bundled with Vite: three + addons + the path tracer come from npm (one deduped
// `three` instance), and the wasm is a Vite asset. The path tracer (Phase 2
// "Render" mode) is imported lazily so it's code-split out of the initial load.

import * as THREE from "three";
import { TrackballControls } from "three/addons/controls/TrackballControls.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { GTAOPass } from "three/addons/postprocessing/GTAOPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import init, { generate } from "./pkg/shell_wasm.js";
import wasmUrl from "./pkg/shell_wasm_bg.wasm?url";

let wasmReady;
function ensureWasm() {
  if (!wasmReady) {
    wasmReady = init({ module_or_path: wasmUrl });
  }
  return wasmReady;
}

const DEFAULTS = {
  w: 2.0,
  d: 0.15,
  t: 1.5,
  n: 5.0,
  aspect: 1.0,
  rib_ax_count: 0,
  rib_ax_amp: 0,
  rib_sp_count: 0,
  rib_sp_amp: 0,
  rib_sharp: 0,
  proj_count: 0,
  proj_rows: 0,
  proj_pos: 0,
  proj_size: 0,
  proj_sharp: 0,
  varix_count: 0,
  varix_amp: 0,
  seed: 0,
  jitter: 0,
  seg_theta: 96,
  seg_phi: 48,
};
const ATTRS = [
  "w",
  "d",
  "t",
  "n",
  "aspect",
  "rib_ax_count",
  "rib_ax_amp",
  "rib_sp_count",
  "rib_sp_amp",
  "rib_sharp",
  "proj_count",
  "proj_rows",
  "proj_pos",
  "proj_size",
  "proj_sharp",
  "varix_count",
  "varix_amp",
  "seed",
  "jitter",
];

// Viewer-side material (becomes Layer-4 params + ID fields later).
const MAT_DEFAULTS = {
  color: 0xe7d8b6,
  roughness: 0.3,
  clearcoat: 0.55,
  clearcoatRoughness: 0.25,
  transmission: 0.0,
  thickness: 0.6,
  ior: 1.45,
  iridescence: 0.0,
  attenuationColor: 0xd9c7a0,
  envMapIntensity: 1.0,
};

class ShellViewer extends HTMLElement {
  static get observedAttributes() {
    return ATTRS;
  }

  constructor() {
    super();
    this.params = { ...DEFAULTS };
    this.matParams = { ...MAT_DEFAULTS };
    this._loaded = false;
    this._framed = false;
    this._mode = "live"; // "live" raster | "hq" path-traced
    this.pathTracer = null;
    this._ptLib = null;
    this.attachShadow({ mode: "open" });
  }

  connectedCallback() {
    const style = document.createElement("style");
    style.textContent = `:host{display:block;width:100%;height:100%;min-height:320px}
      canvas{display:block;width:100%;height:100%;touch-action:none}`;
    this.shadowRoot.append(style);

    this._initThree();

    for (const a of ATTRS) {
      if (this.hasAttribute(a))
        this.params[a] = parseFloat(this.getAttribute(a));
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

  /** Regenerate geometry from shape params. */
  setParams(patch) {
    Object.assign(this.params, patch);
    if (this._loaded) {
      this._rebuild();
      this._resetHQ();
    }
  }

  /** Update material/look without regenerating geometry. */
  setMaterial(patch) {
    Object.assign(this.matParams, patch);
    this._applyMaterial();
    this._resetHQ();
  }

  /**
   * Switch to the progressive path tracer ("photo" render). Lazily code-split.
   * Returns true on success, false (and stays live) on failure.
   */
  async renderHQ() {
    if (!this._loaded) return false;
    try {
      if (!this._ptLib) {
        this._ptLib = await import("three-gpu-pathtracer");
      }
      if (!this.pathTracer) {
        this.pathTracer = new this._ptLib.WebGLPathTracer(this.renderer);
        this.pathTracer.renderScale = 0.75; // accumulate a touch faster
        this.pathTracer.tiles.set(4, 4);
      }
      // Light the tracer with the *same* RoomEnvironment as the live raster (the
      // tracer auto-converts the cube map to an equirect) so the two modes match;
      // show a soft gradient only as a backdrop, not as a light.
      this.scene.environment = this._ensureHQEnv();
      this.scene.background = this._ensureHQBg();
      this._mode = "hq";
      this._applyMaterial(); // drops clearcoat for the tracer
      this.controls.update();
      await this.pathTracer.setScene(this.scene, this.camera);
      return true;
    } catch (e) {
      console.warn(
        "[shell-viewer] path tracer unavailable:",
        e.stack || e.message,
      );
      this._mode = "live";
      return false;
    }
  }

  /** Return to the live raster viewport. */
  stopHQ() {
    this.scene.environment = this.envPMREM;
    this.scene.background = this._liveBg;
    this._mode = "live";
    this._applyMaterial(); // restore clearcoat for live
  }

  /**
   * Render the *same* RoomEnvironment the live raster uses into a cube map, so
   * the path tracer is lit by an identical IBL (it auto-converts a CubeTexture to
   * an equirect internally). This is what makes the HQ render match the live
   * viewport — previously HQ was lit by a separate, mostly-dark gradient, so the
   * directional key left the opposite side in shadow. Built once.
   */
  _ensureHQEnv() {
    if (this.envHQ) return this.envHQ;
    const cubeRT = new THREE.WebGLCubeRenderTarget(256, {
      type: THREE.HalfFloatType,
    });
    const cubeCam = new THREE.CubeCamera(0.1, 100, cubeRT);
    const room = new RoomEnvironment();
    const prevTone = this.renderer.toneMapping;
    this.renderer.toneMapping = THREE.NoToneMapping; // capture linear HDR radiance
    cubeCam.update(this.renderer, room);
    this.renderer.toneMapping = prevTone;
    room.dispose?.();
    this.envHQ = cubeRT.texture; // isCubeTexture === true → tracer matches live IBL
    return this.envHQ;
  }

  /**
   * A soft gradient backdrop for the path-traced still — purely visual (so
   * translucent shells read against something); the lighting comes from
   * `_ensureHQEnv`, not this. Needs the lazily-loaded path-tracer lib.
   */
  _ensureHQBg() {
    if (!this.envHQBg) {
      const tex = new this._ptLib.GradientEquirectTexture();
      tex.topColor.set(0xb8bfc7); // soft neutral studio sky
      tex.bottomColor.set(0x2c2f34); // dim neutral floor
      tex.update();
      this.envHQBg = tex;
    }
    return this.envHQBg;
  }

  /** Rebuild the path-trace scene + restart accumulation (after any change). */
  _resetHQ() {
    if (this._mode === "hq" && this.pathTracer) {
      this.pathTracer.setScene(this.scene, this.camera);
    }
  }

  /** Download the current frame (live or path-traced) as a PNG. */
  saveImage(filename = "shell.png") {
    const a = document.createElement("a");
    a.href = this.renderer.domElement.toDataURL("image/png");
    a.download = filename;
    a.click();
  }

  _initThree() {
    const w = this.clientWidth || 600;
    const h = this.clientHeight || 400;

    this.renderer = new THREE.WebGLRenderer({
      antialias: false,
      preserveDrawingBuffer: true,
    });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    this.renderer.setSize(w, h, false);
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 0.7;
    this.shadowRoot.append(this.renderer.domElement);

    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color(0x0e1116);
    this._liveBg = this.scene.background;

    // Image-based lighting from a generated studio room (zero external asset).
    // Live raster uses the prefiltered PMREM map; the path tracer needs a raw
    // cube/equirect env, so we lazily build a cube version for HQ mode.
    const pmrem = new THREE.PMREMGenerator(this.renderer);
    this.envPMREM = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    this.scene.environment = this.envPMREM;
    this.scene.environmentIntensity = 0.7; // RoomEnvironment is bright; dim the IBL
    pmrem.dispose();
    this.envHQ = null;

    this.camera = new THREE.PerspectiveCamera(40, w / h, 0.01, 100);
    this.camera.position.set(0.7, 1.0, 3.2); // reframed on first build

    // TrackballControls: quaternion-style free rotation — no fixed up-axis, so no
    // gimbal lock when orbiting over the poles (OrbitControls locks there, which
    // is exactly where you want to look down the spire axis).
    this.controls = new TrackballControls(
      this.camera,
      this.renderer.domElement,
    );
    this.controls.rotateSpeed = 3.5;
    this.controls.zoomSpeed = 1.2;
    this.controls.panSpeed = 0.8;
    this.controls.staticMoving = false; // smooth, damped
    this.controls.dynamicDampingFactor = 0.12;
    this.controls.minDistance = 0.5;
    this.controls.maxDistance = 40;
    // Moving the camera resets the path-trace accumulation.
    this.controls.addEventListener("change", () => {
      if (this._mode === "hq" && this.pathTracer)
        this.pathTracer.updateCamera();
    });

    // A soft key light for a crisp specular streak on top of the ambient IBL.
    const key = new THREE.DirectionalLight(0xfff4e6, 0.9);
    key.position.set(4, 6, 5);
    this.scene.add(key);

    this.material = new THREE.MeshPhysicalMaterial({ side: THREE.DoubleSide });
    this._applyMaterial();
    this.mesh = new THREE.Mesh(new THREE.BufferGeometry(), this.material);
    this.scene.add(this.mesh);

    this._initPost(w, h);
  }

  _initPost(w, h) {
    // HDR, multisampled target → bloom works + free MSAA anti-aliasing.
    const size = this.renderer.getDrawingBufferSize(new THREE.Vector2());
    const rt = new THREE.WebGLRenderTarget(size.x, size.y, {
      type: THREE.HalfFloatType,
      samples: 4,
    });
    this.composer = new EffectComposer(this.renderer, rt);
    this.composer.setPixelRatio(window.devicePixelRatio);
    this.composer.setSize(w, h);

    this.composer.addPass(new RenderPass(this.scene, this.camera));

    this.gtao = new GTAOPass(this.scene, this.camera, w, h);
    this.gtao.output = GTAOPass.OUTPUT.Default;
    this.gtao.updateGtaoMaterial({
      radius: 0.25,
      distanceExponent: 1.0,
      scale: 1.0,
      samples: 16,
    });
    this.composer.addPass(this.gtao);

    // Subtle: only genuine HDR highlights (>~1.0 luminance) bloom, so the lit
    // surface itself doesn't glow/read as emissive.
    const bloom = new UnrealBloomPass(new THREE.Vector2(w, h), 0.04, 0.4, 1.2);
    this.composer.addPass(bloom);

    this.composer.addPass(new OutputPass());
  }

  _applyMaterial() {
    const m = this.material;
    const p = this.matParams;
    m.color = new THREE.Color(p.color);
    m.roughness = p.roughness;
    m.metalness = 0.0;
    // Clearcoat renders black in the path tracer (three-gpu-pathtracer bug), so
    // disable it in HQ mode; it stays on for the live raster where it looks good.
    m.clearcoat = this._mode === "hq" ? 0 : p.clearcoat;
    m.clearcoatRoughness = p.clearcoatRoughness;
    m.ior = p.ior;
    m.iridescence = p.iridescence;
    m.iridescenceIOR = 1.3;
    m.envMapIntensity = p.envMapIntensity;
    m.transmission = p.transmission;
    // Volume absorption (attenuation) only when actually translucent — a finite
    // attenuationDistance is applied as absorption by the path tracer even at
    // transmission 0, which would swallow all light and render the shell black.
    if (p.transmission > 0.001) {
      m.thickness = p.thickness;
      m.attenuationColor = new THREE.Color(p.attenuationColor);
      m.attenuationDistance = 2.5; // gentle, unit-scale shell
    } else {
      m.thickness = 0;
      m.attenuationDistance = Infinity;
    }
    m.needsUpdate = true;
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
    const uvs = m.uvs;
    const indices = m.indices;
    m.free();

    const geo = this.mesh.geometry;
    geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    geo.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    geo.setAttribute("uv", new THREE.BufferAttribute(uvs, 2));
    geo.setIndex(new THREE.BufferAttribute(indices, 1));
    geo.computeTangents(); // needs uv + normal; enables normal/detail maps
    geo.computeBoundingSphere();
    // Geometry is already unit-normalised, centred and oriented (spire vertical,
    // cone down, aperture facing +Z) in Rust, so no mesh transform is needed — and
    // the BVH/path tracer get small, precision-friendly coords.

    // Fit the camera to the bounding sphere. The first build uses the canonical
    // front-and-slightly-above pose; later rebuilds keep the user's orbit
    // direction and just re-fit distance/target (so presets stay centred without
    // snapping the view back).
    this._frameObject(!this._framed);
    this._framed = true;
  }

  /** Frame the camera to the mesh's bounding sphere. */
  _frameObject(useDefaultDir) {
    const bs = this.mesh.geometry.boundingSphere;
    if (!bs) return;
    const c = bs.center;
    const r = bs.radius || 1;
    const fov = (this.camera.fov * Math.PI) / 180;
    const dist = (r / Math.sin(fov / 2)) * 1.12;

    let dir;
    if (useDefaultDir) {
      dir = new THREE.Vector3(0.22, 0.32, 1.0).normalize(); // front, a touch above
    } else {
      dir = this.camera.position.clone().sub(this.controls.target);
      if (dir.lengthSq() < 1e-9) dir.set(0.22, 0.32, 1.0);
      dir.normalize();
    }

    this.controls.target.copy(c);
    this.camera.position.copy(c).addScaledVector(dir, dist);
    this.camera.near = Math.max(dist - r * 2, 0.01);
    this.camera.far = dist + r * 4;
    this.camera.updateProjectionMatrix();
    this.controls.update();
  }

  _onResize() {
    const w = this.clientWidth;
    const h = this.clientHeight;
    if (!w || !h) return;
    this.renderer.setSize(w, h, false);
    this.composer.setSize(w, h);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.controls.handleResize(); // TrackballControls caches the element rect
    this._resetHQ();
  }

  _animate() {
    this._raf = requestAnimationFrame(() => this._animate());
    this.controls.update();
    if (this._mode === "hq" && this.pathTracer) {
      this.pathTracer.renderSample();
      this.dispatchEvent(
        new CustomEvent("hq-progress", {
          detail: { samples: Math.floor(this.pathTracer.samples) },
        }),
      );
    } else {
      this.composer.render();
    }
  }
}

customElements.define("shell-viewer", ShellViewer);
export { ShellViewer };
