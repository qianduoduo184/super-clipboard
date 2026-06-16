import { getVersion } from '@tauri-apps/api/app';

let cachedVersion: string | null = null;

export async function getCurrentVersion(): Promise<string> {
  if (cachedVersion) {
    return cachedVersion;
  }

  try {
    cachedVersion = await getVersion();
    return cachedVersion;
  } catch (error) {
    console.error('Failed to get app version:', error);
    return '未知版本';
  }
}
