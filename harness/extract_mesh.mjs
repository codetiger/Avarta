// Mesh-extraction bridge — gets a shell mesh out of the Rust core the *same way
// the web page does*: through the existing wasm-bindgen package in ../web/pkg.
// It does NOT touch the Rust crates and builds nothing — it loads the already-
// built shell_wasm and calls the same `generate(params)` the browser calls.
//
// Usage:  node extract_mesh.mjs '<params-json>'      (params on argv), or
//         echo '<params-json>' | node extract_mesh.mjs
//
// Output (stdout, binary): a little-endian header of four uint32 element counts
//   [nPositions, nNormals, nUvs, nIndices]
// followed by positions(f32), normals(f32), uvs(f32), indices(u32) back to back.
// Python reads this with numpy.frombuffer — no big JSON to parse.

import { readFileSync } from "node:fs";
import { initSync, generate } from "../web/pkg/shell_wasm.js";

// Initialise the wasm synchronously from the same .wasm the website ships.
const wasmPath = new URL("../web/pkg/shell_wasm_bg.wasm", import.meta.url);
initSync({ module: readFileSync(wasmPath) });

function readParams() {
  if (process.argv[2]) return process.argv[2];
  return readFileSync(0, "utf8"); // fd 0 = stdin
}

const params = JSON.parse(readParams());
const mesh = generate(params); // identical call to the web viewer's
const positions = mesh.positions; // Float32Array (fresh copies)
const normals = mesh.normals;
const uvs = mesh.uvs;
const indices = mesh.indices; // Uint32Array
mesh.free();

const header = new Uint32Array([
  positions.length,
  normals.length,
  uvs.length,
  indices.length,
]);

function writeBuf(typedArray) {
  process.stdout.write(
    Buffer.from(typedArray.buffer, typedArray.byteOffset, typedArray.byteLength),
  );
}
writeBuf(header);
writeBuf(positions);
writeBuf(normals);
writeBuf(uvs);
writeBuf(indices);
