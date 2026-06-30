// Build a signed release (NSIS installer + updater artifacts).
//
// The Tauri updater requires the build to sign its artifacts with the private
// key whose public half is pinned in src-tauri/tauri.conf.json ("updater.pubkey").
// Without a key, `tauri build` fails with:
//   "A public key has been found, but no private key. Make sure to set
//    TAURI_SIGNING_PRIVATE_KEY environment variable."
//
// We load that key from ~/.tauri/bizvoice_updater.key (NOT committed to git) and
// expose it via TAURI_SIGNING_PRIVATE_KEY for the build child process only —
// nothing is written to the registry or any persistent store. The key has no
// password, so the signer takes its "Signing without password" path.
//
// Usage: npm run build            (extra args forward, e.g. `npm run build -- --debug`)
import { spawnSync } from 'node:child_process';
import { readFileSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const keyPath = join(homedir(), '.tauri', 'bizvoice_updater.key');

if (!existsSync(keyPath)) {
  console.error(
    `\nUpdater signing key not found at:\n  ${keyPath}\n\n` +
      `Generate one, then replace "updater.pubkey" in src-tauri/tauri.conf.json\n` +
      `with the printed public key:\n  npm run tauri -- signer generate -w "${keyPath}"\n`,
  );
  process.exit(1);
}

const env = {
  ...process.env,
  TAURI_SIGNING_PRIVATE_KEY: readFileSync(keyPath, 'utf8').trim(),
  // Key is password-free. The bundler still prompts for a password unless this is
  // set, so pass an empty string (fine as a process env var — it just can't be a
  // *persisted* Windows registry var, which is why a wrapper rather than a global).
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '',
};

const res = spawnSync('npm', ['run', 'tauri', '--', 'build', ...process.argv.slice(2)], {
  stdio: 'inherit',
  env,
  shell: true, // resolve npm/npm.cmd across platforms
});

process.exit(res.status ?? 1);
