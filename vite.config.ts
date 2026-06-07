import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import electron from 'vite-plugin-electron';
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

export default defineConfig({
  plugins: [
    react(),
    copyVadAssets(),
    electron([
      {
        entry: 'electron/main/index.ts',
        vite: {
          build: {
            outDir: 'dist-electron/main',
            rollupOptions: {
              external: ['@fugood/whisper.node'],
            },
          },
        },
      },
      {
        entry: 'electron/preload/index.ts',
        onstart(options) {
          options.reload();
        },
        vite: {
          build: {
            outDir: 'dist-electron/preload',
          },
        },
      },
    ]),
  ],
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
