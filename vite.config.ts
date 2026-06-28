import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'path';
import { copyFileSync, mkdirSync } from 'fs';

/** Copies @ricky0123/vad-web + onnxruntime-web assets into public/vad/
 *  so they are served locally in both dev and production builds. */
function copyVadAssets() {
  return {
    name: 'copy-vad-assets',
    buildStart() {
      const dest = resolve(__dirname, 'public/vad');
      mkdirSync(dest, { recursive: true });
      const vad = resolve(__dirname, 'node_modules/@ricky0123/vad-web/dist');
      const ort = resolve(__dirname, 'node_modules/onnxruntime-web/dist');
      const files: [string, string][] = [
        [`${vad}/vad.worklet.bundle.min.js`,           `${dest}/vad.worklet.bundle.min.js`],
        [`${vad}/silero_vad_legacy.onnx`,              `${dest}/silero_vad_legacy.onnx`],
        [`${vad}/silero_vad_v5.onnx`,                  `${dest}/silero_vad_v5.onnx`],
        [`${ort}/ort-wasm-simd-threaded.wasm`,         `${dest}/ort-wasm-simd-threaded.wasm`],
        [`${ort}/ort-wasm-simd-threaded.mjs`,          `${dest}/ort-wasm-simd-threaded.mjs`],
        [`${ort}/ort-wasm-simd-threaded.asyncify.wasm`,`${dest}/ort-wasm-simd-threaded.asyncify.wasm`],
        [`${ort}/ort-wasm-simd-threaded.asyncify.mjs`, `${dest}/ort-wasm-simd-threaded.asyncify.mjs`],
      ];
      for (const [src, dst] of files) {
        try { copyFileSync(src, dst); } catch (e) { console.warn('[vad-assets] skip:', src, e); }
      }
    },
  };
}

// onnxruntime-web's dist dir — used to alias vad-web's ORT imports to ORT's
// self-contained ESM "bundle" builds (forward slashes for Vite on Windows).
const ortDist = resolve(__dirname, 'node_modules/onnxruntime-web/dist').replace(/\\/g, '/');

// Tauri-only frontend (Electron has been removed). Tauri's beforeDev/Build
// commands run `vite`/`vite build`; the Rust shell loads the built HTML from
// `dist/` (or the dev server on :5173).
export default defineConfig({
  plugins: [react(), copyVadAssets()],
  resolve: {
    // @ricky0123/vad-web is CommonJS and pulls ORT in via `require("onnxruntime-web")`
    // (non-realtime) and `require("onnxruntime-web/wasm")` (MicVAD/realtime). Their
    // `require` condition resolves to ORT's CJS builds, which reference an EXTERNAL
    // `ort-wasm-*.mjs` loader the Vite optimizer can't emit ("…ort-wasm-simd-threaded
    // .mjs does not exist") — and marking ORT external instead turns the CJS require
    // into an unsupported dynamic require ("require of 'onnxruntime-web/wasm'"). Fix:
    // alias both specifiers to ORT's *.bundle.min.mjs ESM builds (wasm loader inlined),
    // so esbuild bundles a self-contained module. The actual .wasm binary is still
    // fetched at runtime from wasmPaths = VAD_ASSET_PATH ('./vad/', via copyVadAssets).
    // Regex `$` anchors keep the bare match from clobbering the /wasm subpath.
    alias: [
      { find: /^onnxruntime-web\/wasm$/, replacement: `${ortDist}/ort.wasm.bundle.min.mjs` },
      { find: /^onnxruntime-web$/, replacement: `${ortDist}/ort.bundle.min.mjs` },
    ],
  },
  // vad-web is CommonJS (no "module"/"exports" field) so Vite MUST pre-bundle it
  // (CJS→ESM) — otherwise the browser gets raw `exports.…` → "exports is not defined".
  // `include` forces that even though it's reached via a dynamic import().
  optimizeDeps: {
    include: ['@ricky0123/vad-web'],
  },
  // Tauri expects a fixed dev port. Vite must NOT watch src-tauri/ — cargo locks
  // files under target/ during builds and the watcher crashes (EBUSY) on a busy .exe.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    rollupOptions: {
      input: {
        micBar: resolve(__dirname, 'index.html'),
        settings: resolve(__dirname, 'settings.html'),
        login: resolve(__dirname, 'login.html'),
        update: resolve(__dirname, 'update.html'),
      },
    },
  },
});
