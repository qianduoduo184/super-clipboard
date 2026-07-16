export const BACKUP_FILE_EXTENSIONS: readonly ['zip', 'json'];

export type BackendBackupInfo = {
  created_at: string;
  item_count: number;
  version: string;
};

export type BackupInfo = {
  createdAt: string;
  itemCount: number;
  version: string;
};

export function getBackupFormat(info: Pick<BackupInfo, 'version'>): 'ZIP' | '旧版 JSON' | '未知格式';
export function mapBackendBackupInfo(info: BackendBackupInfo): BackupInfo;
