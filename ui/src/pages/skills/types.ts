// Skills 页共用的后端返回结构（计划 §6.1 自 Skills.tsx 拆出）。
export interface AppState {
  app: string;
  enabled: boolean;
  actual: string; // linked | copied | missing | foreign | orphan | absent
  sync_mode: string;
}
export interface SkillView {
  dir: string;
  name: string;
  description: string | null;
  repo: string | null;
  branch: string | null;
  version: string | null;
  latest: string | null;
  size: number;
  managed: boolean;
  updated_at: number;
  apps: AppState[];
}
export interface BackupEntry {
  backup_id: string;
  created_at: number;
  dir: string;
  name: string;
  source_path: string;
}
export interface UnmanagedItem {
  dir: string;
  name: string;
  description: string | null;
  found_in: string[];
  path: string;
}
export interface RepoRow {
  id: number;
  owner: string;
  name: string;
  branch: string;
  enabled: boolean;
}
export interface DiscoveredItem {
  key: string;
  name: string;
  description: string | null;
  directory: string;
  repo_owner: string;
  repo_name: string;
  repo_branch: string;
  installs?: number;
}
export interface UpdateItem {
  dir: string;
  name: string;
  repo: string;
}

export type Tab = "installed" | "matrix" | "unmanaged" | "discover" | "backups" | "repos";
