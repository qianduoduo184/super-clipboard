import { getVersion } from '@tauri-apps/api/app';

let cachedVersion: string | null = null;

export async function getCurrentVersion(): Promise<string> {
  if (cachedVersion) {
    return cachedVersion;
  }

  try {
    const rawVersion = await getVersion();
    // Format: "1.0.3+build.5" -> "v1.0.3 build 5"
    const match = rawVersion.match(/^(\d+\.\d+\.\d+)\+build\.(\d+)$/);
    if (match) {
      cachedVersion = `v${match[1]} build ${match[2]}`;
    } else {
      // Fallback for versions without build number
      cachedVersion = `v${rawVersion}`;
    }
    return cachedVersion;
  } catch (error) {
    console.error('Failed to get app version:', error);
    return '未知版本';
  }
}
