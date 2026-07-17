import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const STABLE_VERSION_PATTERN = /^(\d+)\.(\d+)\.(\d+)$/;
const TAG_VERSION_PATTERN = /(?:^|v)(\d+)\.(\d+)\.(\d+)(?=$|[+-])/;

function parseStableVersion(value) {
  const match = STABLE_VERSION_PATTERN.exec(value);
  if (!match) throw new Error(`Expected a stable semantic version, received: ${value}`);
  return match.slice(1).map(Number);
}

function parseTagVersion(tag) {
  const match = TAG_VERSION_PATTERN.exec(tag);
  return match ? match.slice(1).map(Number) : null;
}

function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
}

function formatVersion(version) {
  return version.join('.');
}

export function chooseNextVersion(repositoryVersion, tags) {
  const current = parseStableVersion(repositoryVersion);
  const released = tags
    .map(parseTagVersion)
    .filter((version) => version !== null)
    .sort(compareVersions)
    .at(-1);

  if (!released || compareVersions(current, released) > 0) {
    return formatVersion(current);
  }

  const next = [...released];
  next[2] += 1;
  return formatVersion(next);
}

function updateJsonVersion(source, version, updateRootPackage = false) {
  const document = JSON.parse(source);
  document.version = version;
  if (updateRootPackage) {
    if (!document.packages?.['']) throw new Error('package-lock.json is missing the root package entry');
    document.packages[''].version = version;
  }
  return `${JSON.stringify(document, null, 2)}\n`;
}

function replaceRequired(source, pattern, replacement, fileName) {
  if (!pattern.test(source)) throw new Error(`Could not locate the application version in ${fileName}`);
  return source.replace(pattern, replacement);
}

export function synchronizeVersionDocuments(documents, version) {
  parseStableVersion(version);

  return {
    packageJson: updateJsonVersion(documents.packageJson, version),
    packageLock: updateJsonVersion(documents.packageLock, version, true),
    tauriConfig: updateJsonVersion(documents.tauriConfig, version),
    cargoToml: replaceRequired(
      documents.cargoToml,
      /(\[package\][\s\S]*?^version\s*=\s*")[^"]+("\s*$)/m,
      `$1${version}$2`,
      'src-tauri/Cargo.toml',
    ),
    cargoLock: replaceRequired(
      documents.cargoLock,
      /(\[\[package\]\]\r?\nname = "super-clipboard"\r?\nversion = ")[^"]+("\s*$)/m,
      `$1${version}$2`,
      'src-tauri/Cargo.lock',
    ),
  };
}

function readManifestVersions(documents) {
  const packageJson = JSON.parse(documents.packageJson);
  const packageLock = JSON.parse(documents.packageLock);
  const tauriConfig = JSON.parse(documents.tauriConfig);
  const cargoTomlVersion = /\[package\][\s\S]*?^version\s*=\s*"([^"]+)"\s*$/m.exec(documents.cargoToml)?.[1];
  const cargoLockVersion = /\[\[package\]\]\r?\nname = "super-clipboard"\r?\nversion = "([^"]+)"\s*$/m.exec(documents.cargoLock)?.[1];

  return [
    packageJson.version,
    packageLock.version,
    packageLock.packages?.['']?.version,
    tauriConfig.version,
    cargoTomlVersion,
    cargoLockVersion,
  ];
}

async function main() {
  const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const paths = {
    packageJson: resolve(projectRoot, 'package.json'),
    packageLock: resolve(projectRoot, 'package-lock.json'),
    tauriConfig: resolve(projectRoot, 'src-tauri/tauri.conf.json'),
    cargoToml: resolve(projectRoot, 'src-tauri/Cargo.toml'),
    cargoLock: resolve(projectRoot, 'src-tauri/Cargo.lock'),
  };
  const entries = await Promise.all(
    Object.entries(paths).map(async ([key, path]) => [key, await readFile(path, 'utf8')]),
  );
  const documents = Object.fromEntries(entries);
  const versions = readManifestVersions(documents);
  const uniqueVersions = new Set(versions);
  if (uniqueVersions.size !== 1 || versions.some((version) => !version)) {
    throw new Error(`Application versions are not synchronized: ${versions.join(', ')}`);
  }

  const tags = execFileSync('git', ['tag', '--list'], { cwd: projectRoot, encoding: 'utf8' })
    .split(/\r?\n/)
    .filter(Boolean);
  const nextVersion = chooseNextVersion(versions[0], tags);
  const updated = synchronizeVersionDocuments(documents, nextVersion);

  await Promise.all(
    Object.entries(paths).map(([key, path]) => writeFile(path, updated[key], 'utf8')),
  );
  process.stdout.write(nextVersion);
}

const scriptPath = process.argv[1] ? resolve(process.argv[1]) : '';
if (scriptPath === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
