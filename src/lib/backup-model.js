export const BACKUP_FILE_EXTENSIONS = Object.freeze(['zip', 'json']);

export function getBackupFormat(info) {
  if (info.version === '2') return 'ZIP';
  if (info.version === '1.0') return '旧版 JSON';
  return '未知格式';
}

export function mapBackendBackupInfo(info) {
  return {
    createdAt: info.created_at,
    itemCount: info.item_count,
    version: info.version,
  };
}
