// 应用更新检测的前端唯一入路（fix:update-check-and-install）。
//
// 口径（与服务层 src-tauri/src/services/update.rs 一致，别在这儿另定规则）：
// - 检测失败 ≠ 无更新：后端把失败原因放在 error 字段里返回，自动检查时**不显示标签也不报错**，
//   只有用户手动点「检查更新」才把 error 说出来。
// - 下载走通用后台任务队列（任务托盘那条通道），这里只拿任务 id 去轮询进度。
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./ipc";

export const UPDATE_KEY = ["update-check"];

/** 双公仓发布页（铁律：URL 集中定义，不散落字面量；与 Rust services/update.rs 的
 *  GH/GITEE OWNER·REPO 同一对公开仓）。版本徽标与更新面板都从这里取。 */
export const RELEASE_PAGES = {
  github: "https://github.com/Lowkey119/llm-switch/releases",
  gitee: "https://gitee.com/lowkey110/llm-switch/releases",
} as const;

/** 一个可下载的安装包（与 Rust `update::Asset` 同形）。 */
export interface UpdateAsset {
  name: string;
  url: string;
  size: number;
  /** CI 随包出的 SHA256；缺校验和的包后端会拒绝自动安装。 */
  sha256: string | null;
  platform_match: boolean;
  /** Gitee 直链（附件接口不开放，从 Release 正文解析），没有就是 null。 */
  gitee_url: string | null;
}

export interface UpdateInfo {
  current: string;
  tag: string;
  version: string;
  notes: string;
  source: string;
  published: string | null;
  assets: UpdateAsset[];
  has_update: boolean;
  page_url: string;
  /** 两个源都失败时才有值；自动检查时保持静默。 */
  error?: string;
}

/** 顶栏与设置页共用同一份检测结果（同键订阅，不各查各的）。 */
export function useUpdateCheck(enabled = true) {
  return useQuery({
    queryKey: UPDATE_KEY,
    queryFn: () => ipc<UpdateInfo>("update_check"),
    enabled,
    /* 不排后台轮询：版本不是实时数据，而 GitHub 匿名 API 限流 60 次/小时——
       按 POLL 最低档（60s）轮询会一小时把额度吃光，之后真发新版也检不出来。
       进应用查一次（staleTime 内复用），要再查就在更新面板里手动点「重新检查」。 */
    staleTime: 30 * 60 * 1000,
    retry: 0,
  });
}

/** 手动「重新检查」：直接取一次并写回同一份缓存（顶栏标签立刻跟着变）。
 *  用 fetchQuery 而不是 refetchQueries——后者的返回类型里拿不到数据，
 *  而手动检查恰恰要把结果原样报给用户（成功/无更新/失败原因）。 */
export function useRefreshUpdate() {
  const qc = useQueryClient();
  return () => qc.fetchQuery({ queryKey: UPDATE_KEY, queryFn: () => ipc<UpdateInfo>("update_check") });
}

/** 下载并校验：返回任务 id，进度由任务托盘那条通道走。 */
export function startUpdateDownload(a: UpdateAsset) {
  return ipc<string>("update_download", { url: a.url, sha256: a.sha256, name: a.name });
}

export function launchUpdateInstall(path: string) {
  return ipc<string>("update_install", { path });
}

/** 尊重系统「减少动态效果」：要求减少时不呼吸，固定显示新版本号。 */
export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    !!window.matchMedia &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}
