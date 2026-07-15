export const BACKUP_FILE_EXTENSIONS = Object.freeze(['zip', 'json']);

export function getBackupFormat(path) {
  const extension = String(path).split('.').pop()?.toLowerCase();
  if (extension === 'zip') return 'ZIP';
  if (extension === 'json') return '旧版 JSON';
  return '未知格式';
}

export function mapBackendBackupInfo(info) {
  return {
    createdAt: info.created_at,
    itemCount: info.item_count,
    version: info.version,
  };
}
