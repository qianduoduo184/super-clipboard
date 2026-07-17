import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { chooseNextVersion, synchronizeVersionDocuments } from './versioning.mjs';

test('increments patch after the highest released semantic version', () => {
  const nextVersion = chooseNextVersion('1.1.1', [
    'super-clipboard-v1.1.1+1-1',
    'super-clipboard-v1.1.0+1-1',
    'v1.0.4',
  ]);

  assert.equal(nextVersion, '1.1.2');
});

test('keeps an explicit repository version that is newer than every release tag', () => {
  const nextVersion = chooseNextVersion('1.2.0', [
    'super-clipboard-v1.1.5',
    'not-a-version',
  ]);

  assert.equal(nextVersion, '1.2.0');
});

test('synchronizes the release version across npm, Tauri, and Cargo manifests', () => {
  const updated = synchronizeVersionDocuments({
    packageJson: '{"name":"super-clipboard","version":"1.1.1"}\n',
    packageLock: '{"name":"super-clipboard","version":"1.1.1","packages":{"":{"version":"1.1.1"}}}\n',
    tauriConfig: '{"productName":"super-clipboard","version":"1.1.1"}\n',
    cargoToml: '[package]\nname = "super-clipboard"\nversion = "1.1.1"\n\n[dependencies]\n',
    cargoLock: '[[package]]\nname = "super-clipboard"\nversion = "1.1.1"\ndependencies = []\n',
  }, '1.1.2');

  assert.equal(JSON.parse(updated.packageJson).version, '1.1.2');
  assert.equal(JSON.parse(updated.packageLock).version, '1.1.2');
  assert.equal(JSON.parse(updated.packageLock).packages[''].version, '1.1.2');
  assert.equal(JSON.parse(updated.tauriConfig).version, '1.1.2');
  assert.match(updated.cargoToml, /version = "1\.1\.2"/);
  assert.match(updated.cargoLock, /version = "1\.1\.2"/);
});

test('release workflow uses patch versions and persists the successful release version', async () => {
  const workflow = await readFile(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');

  assert.match(workflow, /node scripts\/versioning\.mjs/);
  assert.match(workflow, /tagName: super-clipboard-v__VERSION__\s*$/m);
  assert.match(workflow, /name: Persist released version/);
  assert.doesNotMatch(workflow, /fullVersion|Calculate version-specific build number/);
});
