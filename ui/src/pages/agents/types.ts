// 代理页共用的后端返回结构与时间格式化。
// 单独成文件是因为列表、详情、备份三处都要用同一份口径，之前全挤在 Agents.tsx 里。

export interface FileEval {
  path: string;
  format: string;
  exists: boolean;
  managed: boolean;
  note: string;
  /** 内置默认路径（路径自定义的身份键）；overridden 为真时与 path 不同。 */
  default_path: string;
  overridden: boolean;
}

export interface Sel {
  default: string | null;
  tiers: Record<string, string>;
}

export interface ClientRow {
  app: string;
  label: string;
  managed: boolean;
  pending: boolean;
  files: FileEval[];
  applied_at: number | null;
  reverted_at: number | null;
  latest_backup: string | null;
  /** 模型现状：current 是配置文件里的真值，desired 是我们上次写的期望。 */
  model: { current: Sel; desired: Sel; note?: string };
  error?: string;
}

export interface TakeoverStatus {
  endpoint: { base_root: string; api_root: string; models: string[] };
  /** 家目录：路径显示折叠成 ~/ 与「越界」风险判断都用它。 */
  home: string;
  clients: ClientRow[];
}

export interface BackupEntry {
  backup_id: string;
  app: string;
  created_at: number;
  endpoint: string;
  files: string[];
}

export function when(ms: number | null): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}
