// <shell-viewer> — a custom element that renders a shell mesh.
//
// Bundled with Vite: three + addons + the path tracer come from npm (one deduped
// `three` instance), and the wasm is a Vite asset. The path tracer (Phase 2
// "Render" mode) is imported lazily so it's code-split out of the initial load.

import * as THREE from "three";
import { TrackballControls } from "three/addons/controls/TrackballControls.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import { RGBELoader } from "three/addons/loaders/RGBELoader.js";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { GTAOPass } from "three/addons/postprocessing/GTAOPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import init, { generate, param_ranges, pigment_ranges } from "./pkg/shell_wasm.js";
import wasmUrl from "./pkg/shell_wasm_bg.wasm?url";
// A real (CC0) equirectangular HDRI — drives both the IBL reflections and the
// visible background, so the scene looks photographed rather than floating on a
// flat colour. Swap this file to change the environment.
import hdrUrl from "./assets/environment.hdr?url";

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
  // Layer 3 — pigmentation (reaction–diffusion). Defaults show a gentle pattern
  // on first load; species presets and the UI override these.
  pig_regime: 1,
  pig_scale: 0.5,
  pig_contrast: 0.6,
  pig_density: 0.5,
  pig_angle: 0.5,
  pig_irregularity: 0.15,
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
  "pig_regime",
  "pig_scale",
  "pig_contrast",
  "pig_density",
  "pig_angle",
  "pig_irregularity",
];

// Viewer-side material finish (Layer-4 surface, sans colour — the colour now
// comes from the pigment texture, so `color` stays white and the map carries it).
const MAT_DEFAULTS = {
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

// Layer-4 palette: the colours the reaction–diffusion pigment field is mapped
// through (0 = base ground, mid = accent, 1 = pattern). Applied viewer-side so
// recolouring is a cheap texture re-bake with no geometry/RD rerun.
const PALETTE_DEFAULTS = {
  base: "#efe3c8", // unpigmented shell ground (cream)
  accent: "#b9793f", // mid-tone transition
  pattern: "#6f3d1d", // pigment (warm brown)
};

class ShellViewer extends HTMLElement {
  static get observedAttributes() {
    return ATTRS;
  }

  constructor() {
    super();
    this.params = { ...DEFAULTS };
    this.matParams = { ...MAT_DEFAULTS };
    this.palette = { ...PALETTE_DEFAULTS };
    this._loaded = false;
    this._framed = false;
    this._mode = "live"; // "live" raster | "hq" path-traced
    this._renderMode = "solid"; // "solid" | "wireframe" | "solid+wireframe"
    this.pathTracer = null;
    this._ptLib = null;
    // Cached pigment field (from the last generate) so palette changes re-bake
    // the texture without re-running the reaction–diffusion sim.
    this._pigment = null;
    this._pigW = 0;
    this._pigH = 0;
    this._pigTex = null;
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

  /**
   * The Rust-defined parameter range table — the single source of truth for
   * every slider's min/max/step/default. Resolves after the wasm module loads.
   * Returns an array of `{ key, label, min, max, step, default, integer }`.
   */
  async paramRanges() {
    await ensureWasm();
    return param_ranges();
  }

  /**
   * The Rust-defined Layer-3 pigmentation range table — drives the Appearance
   * panel's pigmentation sliders. Same shape as `paramRanges()`.
   */
  async pigmentRanges() {
    await ensureWasm();
    return pigment_ranges();
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
   * Update the Layer-4 pigment palette (base / accent / pattern colours).
   * Look-only: re-bakes the pigment texture from the cached field with no
   * geometry rebuild or reaction–diffusion rerun.
   */
  setPalette(patch) {
    Object.assign(this.palette, patch);
    this._bakePigmentTexture();
    this._resetHQ();
  }

  /**
   * Choose the live display mode: "solid" (default), "wireframe" (edges only),
   * or "solid+wireframe" (shaded surface with a faint edge overlay). Look-only —
   * no geometry regeneration. The path-traced still always renders the solid
   * surface regardless (see renderHQ).
   */
  setRenderMode(mode) {
    this._renderMode = mode;
    if (this._mode !== "hq") this._applyRenderMode();
    this._resetHQ();
  }

  /** Apply the current display mode by toggling the solid mesh / wire overlay. */
  _applyRenderMode() {
    this.mesh.visible = this._renderMode !== "wireframe";
    this.wire.visible = this._renderMode !== "solid";
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
      // Light the tracer with the *same* environment as the live raster — the
      // real HDRI equirect once loaded (used directly), otherwise the built-from-
      // RoomEnvironment cube map — so the two modes match. The background is the
      // same environment, so the path-traced still keeps the photographed backdrop.
      this.scene.environment = this._ensureHQEnv();
      this.scene.background = this._ensureHQBg();
      this._mode = "hq";
      // The path tracer only traces the solid mesh (it ignores the line overlay),
      // so always render the shaded surface for the still — even from a wireframe view.
      this.mesh.visible = true;
      this.wire.visible = false;
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
    this._applyRenderMode(); // restore the chosen solid/wireframe display
  }

  /**
   * Load the real HDRI environment map and swap it in for both the IBL (replacing
   * the bootstrap RoomEnvironment) and the visible background, so the shell sits
   * in a photographed scene. The texture is equirectangular, so the path tracer
   * can light from it directly. Falls back silently to the RoomEnvironment IBL +
   * flat background if the asset can't be fetched (e.g. offline).
   */
  _loadEnvironment() {
    new RGBELoader().load(
      hdrUrl,
      (hdr) => {
        hdr.mapping = THREE.EquirectangularReflectionMapping;
        const pmrem = new THREE.PMREMGenerator(this.renderer);
        const prefiltered = pmrem.fromEquirectangular(hdr).texture;
        pmrem.dispose();

        this.envPMREM?.dispose?.();
        this.envPMREM = prefiltered; // prefiltered IBL for the live raster
        this.envEquirect = hdr; // raw equirect for the path tracer + background

        // A real, visible backdrop — softened and dimmed a touch so it frames the
        // shell rather than competing with it. Lighting still comes from the full
        // (un-blurred) PMREM/equirect, only the displayed background is softened.
        this.scene.backgroundBlurriness = 0.06;
        this.scene.backgroundIntensity = 0.75;
        this._liveBg = hdr;

        // Only repaint the live scene here; if the user is already in HQ mode the
        // tracer is rebuilt below and picks up the equirect via _ensureHQEnv/Bg.
        if (this._mode === "live") {
          this.scene.environment = this.envPMREM;
          this.scene.background = this._liveBg;
        }
        this._resetHQ();
      },
      undefined,
      (err) =>
        console.warn(
          "[shell-viewer] HDRI environment failed to load; keeping procedural fallback:",
          err,
        ),
    );
  }

  /**
   * Render the *same* RoomEnvironment the live raster uses into a cube map, so
   * the path tracer is lit by an identical IBL (it auto-converts a CubeTexture to
   * an equirect internally). This is what makes the HQ render match the live
   * viewport — previously HQ was lit by a separate, mostly-dark gradient, so the
   * directional key left the opposite side in shadow. Built once.
   */
  _ensureHQEnv() {
    if (this.envEquirect) return this.envEquirect; // real HDRI: tracer uses it directly
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
    if (this.envEquirect) return this.envEquirect; // show the real environment behind the shell
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

  /**
   * Render a set of parameter presets to small PNG data-URLs for the species
   * browser. Uses a throwaway offscreen renderer (separate GL context) with
   * simple lights — no IBL/post — so it's cheap; the context is freed afterwards.
   * Returns an array of data-URL strings (or null for any preset that fails),
   * aligned with `paramSets`.
   */
  async makeThumbnails(paramSets, size = 128) {
    await ensureWasm();
    const r = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    r.setPixelRatio(1);
    r.setSize(size, size, false);
    r.toneMapping = THREE.ACESFilmicToneMapping;
    r.toneMappingExposure = 0.85;

    const scene = new THREE.Scene();
    scene.add(new THREE.HemisphereLight(0xffffff, 0x36383f, 1.15));
    const key = new THREE.DirectionalLight(0xfff4e6, 1.4);
    key.position.set(3, 5, 4);
    scene.add(key);
    const cam = new THREE.PerspectiveCamera(40, 1, 0.01, 100);
    const mat = new THREE.MeshPhysicalMaterial({
      side: THREE.DoubleSide,
      color: 0xe7d8b6,
      roughness: 0.4,
      metalness: 0.0,
      clearcoat: 0.3,
    });
    const mesh = new THREE.Mesh(new THREE.BufferGeometry(), mat);
    scene.add(mesh);

    const out = [];
    for (const params of paramSets) {
      let m;
      try {
        // Lower tessellation than the live view — thumbnails are small, and some
        // species (high n + ornament) are very dense at full resolution.
        // pig_regime 0 (solid) short-circuits the reaction–diffusion sim: these
        // shape-only thumbnails carry no UVs/colour map, so the field is unused.
        m = generate({ ...DEFAULTS, seg_theta: 64, seg_phi: 32, ...params, pig_regime: 0 });
      } catch (e) {
        console.warn("[shell-viewer] thumbnail generate failed:", e);
        out.push(null);
        continue;
      }
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.BufferAttribute(m.positions, 3));
      geo.setAttribute("normal", new THREE.BufferAttribute(m.normals, 3));
      geo.setIndex(new THREE.BufferAttribute(m.indices, 1));
      m.free();
      geo.computeBoundingSphere();
      mesh.geometry.dispose();
      mesh.geometry = geo;

      // Frame the camera to the mesh from the canonical front-and-above pose.
      const bs = geo.boundingSphere;
      const rad = bs.radius || 1;
      const fov = (cam.fov * Math.PI) / 180;
      const dist = (rad / Math.sin(fov / 2)) * 1.15;
      const dir = new THREE.Vector3(0.22, 0.32, 1.0).normalize();
      cam.position.copy(bs.center).addScaledVector(dir, dist);
      cam.near = Math.max(dist - rad * 2, 0.01);
      cam.far = dist + rad * 4;
      cam.lookAt(bs.center);
      cam.updateProjectionMatrix();

      r.render(scene, cam);
      out.push(r.domElement.toDataURL("image/png"));
    }

    mesh.geometry.dispose();
    mat.dispose();
    r.dispose();
    r.forceContextLoss?.();
    return out;
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

    // Image-based lighting. Bootstrap with a generated studio room (zero-asset,
    // available instantly) so the shell is lit the moment the scene appears, then
    // asynchronously swap in a real HDRI environment map for true-to-life
    // reflections and a non-plain background. Live raster uses a prefiltered PMREM
    // map; the path tracer needs a raw equirect env, which the HDRI provides
    // directly (and we fall back to a built cube map if the HDRI never loads).
    const pmrem = new THREE.PMREMGenerator(this.renderer);
    this.envPMREM = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    this.scene.environment = this.envPMREM;
    this.scene.environmentIntensity = 0.7; // RoomEnvironment is bright; dim the IBL
    pmrem.dispose();
    this.envEquirect = null; // the raw HDRI, set once it loads
    this.envHQ = null;
    this._loadEnvironment();

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
    // Default to a vertically-flipped orientation: the Rust geometry arrives spire-
    // up / aperture-facing-+Z, so a 180° turn about Z points the spire down while
    // keeping the aperture toward the camera (rotation, not a negative scale, so
    // normals/winding stay correct and the path tracer reads it the same way).
    this.mesh.rotation.z = Math.PI;
    this.scene.add(this.mesh);

    // A line overlay for the wireframe / solid+wireframe display modes. Shares the
    // scene with the solid mesh; its geometry is (re)built from the mesh whenever
    // the shape changes (see _rebuild) — it starts empty so we never construct a
    // WireframeGeometry from the placeholder geometry (which has no positions).
    // Hidden until a wireframe mode is selected.
    this.wireMat = new THREE.LineBasicMaterial({
      color: 0x8b97a8,
      transparent: true,
      opacity: 0.55,
    });
    this.wire = new THREE.LineSegments(new THREE.BufferGeometry(), this.wireMat);
    this.wire.visible = false;
    this.wire.rotation.z = Math.PI; // match the solid mesh's vertical flip
    this.scene.add(this.wire);

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
    // The pigment texture carries all surface colour, so the base colour stays
    // white (a map multiplies the colour — any tint would shift the palette).
    m.map = this._pigTex || null;
    m.color.set(0xffffff);
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

  /**
   * Bake the cached reaction–diffusion pigment field into the material's colour
   * map, mapping the 0..255 pigment scalar through the Layer-4 palette
   * (base → accent → pattern). The field's axes are the growth axes, so the
   * mesh's own UVs (u=θ along the coil, v=φ around the lip) map it with no
   * distortion: clamp along the coil, repeat around the periodic lip. Works in
   * both the live raster and the path tracer (both honour `material.map`).
   */
  _bakePigmentTexture() {
    if (!this._pigment || !this._pigW || !this._pigH) return;
    const w = this._pigW;
    const h = this._pigH;

    // 256-entry colour LUT, lerped in linear space then encoded to sRGB bytes.
    const base = new THREE.Color(this.palette.base);
    const accent = new THREE.Color(this.palette.accent);
    const pattern = new THREE.Color(this.palette.pattern);
    const lut = new Uint8Array(256 * 3);
    const c = new THREE.Color();
    for (let i = 0; i < 256; i++) {
      const t = i / 255;
      if (t < 0.5) c.copy(base).lerp(accent, t / 0.5);
      else c.copy(accent).lerp(pattern, (t - 0.5) / 0.5);
      c.convertLinearToSRGB();
      lut[i * 3] = Math.round(c.r * 255);
      lut[i * 3 + 1] = Math.round(c.g * 255);
      lut[i * 3 + 2] = Math.round(c.b * 255);
    }

    const pig = this._pigment;
    const data = new Uint8Array(w * h * 4);
    for (let p = 0; p < w * h; p++) {
      const v = pig[p];
      data[p * 4] = lut[v * 3];
      data[p * 4 + 1] = lut[v * 3 + 1];
      data[p * 4 + 2] = lut[v * 3 + 2];
      data[p * 4 + 3] = 255;
    }

    const tex = new THREE.DataTexture(data, w, h, THREE.RGBAFormat);
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.wrapS = THREE.ClampToEdgeWrapping; // u = θ along the coil
    tex.wrapT = THREE.RepeatWrapping; // v = φ around the lip (closed loop)
    tex.generateMipmaps = true;
    tex.minFilter = THREE.LinearMipmapLinearFilter;
    tex.magFilter = THREE.LinearFilter;
    if (this.renderer) tex.anisotropy = this.renderer.capabilities.getMaxAnisotropy();
    tex.needsUpdate = true;

    if (this._pigTex) this._pigTex.dispose();
    this._pigTex = tex;
    this.material.map = tex;
    this.material.color.set(0xffffff);
    this.material.needsUpdate = true;
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
    // Read the pigment field before freeing the wasm-side mesh, then cache it so
    // palette tweaks can re-bake the texture without re-running generate().
    this._pigment = m.pigment;
    this._pigW = m.pig_w;
    this._pigH = m.pig_h;
    m.free();

    const geo = this.mesh.geometry;
    geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    geo.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    geo.setAttribute("uv", new THREE.BufferAttribute(uvs, 2));
    geo.setIndex(new THREE.BufferAttribute(indices, 1));
    geo.computeTangents(); // needs uv + normal; enables normal/detail maps
    geo.computeBoundingSphere();

    // Re-bake the pigment colour map for the freshly generated field.
    this._bakePigmentTexture();

    // Keep the wireframe overlay in sync with the new geometry (cheap when hidden).
    this.wire.geometry.dispose();
    this.wire.geometry = new THREE.WireframeGeometry(geo);
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
    // Use the world-space centre: the mesh carries a 180° flip, so a non-origin
    // local centre lands on the opposite side once transformed. Framing the local
    // centre directly would aim the camera at empty space and push the shell off
    // to one side of the view.
    this.mesh.updateMatrixWorld();
    const c = bs.center.clone().applyMatrix4(this.mesh.matrixWorld);
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
