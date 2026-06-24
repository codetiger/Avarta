import fs from "node:fs";
import path from "node:path";
import { defineConfig } from "vite";

// DEV ONLY: serve the shared design system at /design-system/ during `npm run dev`.
// In production the aggregator (website-cicd) publishes it at that exact path, so the
// <link href="/design-system/..."> in index.html is a plain runtime link with no copy
// committed here. Locally there's no such root, so this streams the files straight from
// the design-system folder in the website-cicd repo (always in sync, no duplicate).
// Build/preview are untouched. Override the location with DESIGN_SYSTEM_DIR.
function designSystemDev() {
  const MIME = {
    ".css": "text/css", ".html": "text/html", ".json": "application/json",
    ".js": "text/javascript", ".mjs": "text/javascript", ".svg": "image/svg+xml",
  };
  const dir = [
    process.env.DESIGN_SYSTEM_DIR,                                   // explicit override
    path.resolve(process.cwd(), "../../../design-system"),           // submodule: website-cicd/projects/avarta/web
    path.resolve(process.cwd(), "../../website-cicd/design-system"), // standalone clone beside website-cicd
  ].filter(Boolean).find((d) => fs.existsSync(path.join(d, "design-system.css")));
  return {
    name: "design-system-dev",
    apply: "serve",
    configureServer(server) {
      if (!dir) {
        server.config.logger.warn("[design-system] not found — set DESIGN_SYSTEM_DIR; /design-system/ will 404 locally");
        return;
      }
      server.config.logger.info(`[design-system] serving /design-system/ from ${dir}`);
      server.middlewares.use("/design-system", (req, res, next) => {
        const rel = decodeURIComponent((req.url || "/").split("?")[0]);
        const file = path.join(dir, rel === "/" ? "index.html" : rel);
        if (!path.resolve(file).startsWith(path.resolve(dir))) return next(); // traversal guard
        fs.readFile(file, (err, data) => {
          if (err) return next();
          res.setHeader("Content-Type", MIME[path.extname(file)] || "application/octet-stream");
          res.end(data);
        });
      });
    },
  };
}

// Relative base so the built app works under the "/avarta/" subpath
// (codetiger.in/avarta/) and when its assets are referenced from elsewhere.
export default defineConfig({
  base: "./",
  plugins: [designSystemDev()],
  server: { port: 8090, host: true },
  optimizeDeps: {
    // Pre-bundle three.js + the addons imported by avarta-viewer.js at startup so Vite
    // doesn't discover them lazily and re-optimize mid-session — the cause of the
    // intermittent 504 "Outdated Optimize Dep".
    include: [
      "three",
      "three/addons/controls/TrackballControls.js",
      "three/addons/environments/RoomEnvironment.js",
      "three/addons/loaders/RGBELoader.js",
      "three/addons/postprocessing/EffectComposer.js",
      "three/addons/postprocessing/RenderPass.js",
      "three/addons/postprocessing/GTAOPass.js",
      "three/addons/postprocessing/UnrealBloomPass.js",
      "three/addons/postprocessing/OutputPass.js",
    ],
  },
  build: { outDir: "dist", target: "es2022", emptyOutDir: true },
});
