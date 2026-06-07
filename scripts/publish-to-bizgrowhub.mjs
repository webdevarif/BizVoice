// Post-build publisher.
// Runs after `electron-builder` (see package.json "build"). Copies the freshly
// built Windows installer into the BizGrowHub web app's public/downloads folder
// (a permanent version-wise copy + a stable "latest" copy that the download
// button / in-app updater point at) and refreshes bizvoice-latest.json.
//
// Override the BizGrowHub location with the BIZGROWHUB_DIR env var.
//
// NOTE: installers are ~150–200 MB. Keep apps/web/public/downloads OUT of git
// (it's gitignored) — GitHub rejects files > 100 MB. For production hosting use
// GitHub Releases or object storage instead of shipping the binary in the repo.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const { version, productName = 'BizVoice' } = (() => {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
  return { version: pkg.version, productName: pkg.build?.productName };
})();

const BIZGROWHUB_DIR =
  process.env.BIZGROWHUB_DIR || 'C:/Users/WE/.openclaw/workspace/projects/bizgrowhub';
const publicDir = path.join(BIZGROWHUB_DIR, 'apps', 'web', 'public');

// electron-builder NSIS output: release/<version>/<productName> Setup <version>.exe
const installer = path.join(root, 'release', version, `${productName} Setup ${version}.exe`);

if (!fs.existsSync(installer)) {
  console.error(`[publish] ✗ installer not found: ${installer}`);
  process.exit(1);
}
if (!fs.existsSync(publicDir)) {
  console.warn(`[publish] ⚠ BizGrowHub not found at ${publicDir} — skipping publish.`);
  console.warn('[publish]   Set BIZGROWHUB_DIR to the bizgrowhub repo to enable it.');
  process.exit(0);
}

const downloadsDir = path.join(publicDir, 'downloads');
fs.mkdirSync(path.join(downloadsDir, version), { recursive: true });

const versionedName = `BizVoice-Setup-${version}.exe`;
const versionedDest = path.join(downloadsDir, version, versionedName); // archive, version-wise
const latestDest = path.join(downloadsDir, 'BizVoice-Setup.exe');       // stable download-button target
fs.copyFileSync(installer, versionedDest);
fs.copyFileSync(installer, latestDest);

// Refresh the manifest the in-app updater + landing download button read.
const manifestPath = path.join(publicDir, 'bizvoice-latest.json');
let manifest = {};
try { manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')); } catch { /* first run */ }
manifest.version = version;
manifest.releasedAt = new Date().toISOString().slice(0, 10);
manifest.mandatory = manifest.mandatory ?? false;
manifest.downloadUrl = 'https://bizgrowhub.shop/downloads/BizVoice-Setup.exe';
fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');

const sizeMB = (fs.statSync(installer).size / 1048576).toFixed(0);
console.log(`[publish] ✓ v${version} (${sizeMB} MB) published to BizGrowHub`);
console.log(`[publish]   archive : ${versionedDest}`);
console.log(`[publish]   latest  : ${latestDest}`);
console.log(`[publish]   manifest: ${manifestPath} (downloadUrl → ${manifest.downloadUrl})`);
