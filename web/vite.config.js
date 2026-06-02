import { defineConfig } from "vite";

// Relative base so the built app works under the GitHub Pages "/avarta/" subpath
// and when its assets are referenced from elsewhere.
export default defineConfig({
  base: "./",
  server: { port: 8090, host: true },
  build: { outDir: "dist", target: "es2022", emptyOutDir: true },
});
