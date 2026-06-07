#!/usr/bin/env node
// Release pipeline wrapper.
//
// Problem this solves: GH_TOKEN stored as a Windows User-level env var only
// reaches NEW shells. Older PowerShell windows don't inherit it, so
// `electron-builder --publish always` fails with "GitHub Personal Access
// Token is not set" — silent in scrolling build output, easy to miss.
//
// What it does: before running the pipeline, if GH_TOKEN isn't in process.env,
// read it directly from the User-level registry via PowerShell and inject it.
// Then run version bump → vite build → electron-builder → bizgrowhub publish.
// Any failure aborts with a clear error.

import { execSync, spawnSync } from 'node:child_process';
import { platform } from 'node:os';

function loadGhTokenFromUserEnv() {
  if (process.env.GH_TOKEN) {
    log(`GH_TOKEN already in process env (length=${process.env.GH_TOKEN.length})`);
    return;
  }
  if (platform() !== 'win32') return;
  try {
    const out = execSync(
      `powershell -NoProfile -Command "[System.Environment]::GetEnvironmentVariable('GH_TOKEN','User')"`,
      { encoding: 'utf-8', stdio: ['ignore', 'pipe', 'ignore'] },
    ).trim();
    if (out) {
      process.env.GH_TOKEN = out;
      log(`Loaded GH_TOKEN from User env (length=${out.length})`);
    }
  } catch {
    // PowerShell or registry read failed — fall through to the missing-token check
  }
}

function log(msg) {
  console.log(`[release] ${msg}`);
}

function run(cmd, args) {
  log(`> ${cmd} ${args.join(' ')}`);
  const result = spawnSync(cmd, args, { stdio: 'inherit', shell: true, env: process.env });
  if (result.status !== 0) {
    console.error(`[release] FAILED: ${cmd} exited with code ${result.status}`);
    process.exit(result.status ?? 1);
  }
}

loadGhTokenFromUserEnv();

if (!process.env.GH_TOKEN) {
  console.error('');
  console.error('[release] ERROR: GH_TOKEN is not set anywhere reachable.');
  console.error('');
  console.error('  Set it once with this PowerShell command:');
  console.error(`    [System.Environment]::SetEnvironmentVariable('GH_TOKEN', 'ghp_xxxx...', 'User')`);
  console.error('');
  console.error('  Then re-run `pnpm run release`. (No shell restart needed —');
  console.error('  this script reads User-scope env vars directly.)');
  console.error('');
  process.exit(1);
}

run('npm', ['version', 'patch', '--no-git-tag-version']);
run('npx', ['vite', 'build']);
run('npx', ['electron-builder', '--publish', 'always']);
run('node', ['scripts/publish-to-bizgrowhub.mjs']);

log('Release pipeline completed successfully.');
