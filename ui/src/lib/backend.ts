/**
 * 后端展示口径（U2 徽标铁律）：徽标只认引擎名，DB 真名不动、tooltip 留全名。
 * halogen-thunderbolt 这类实例后缀在徽标里是噪音——用户口径只显示 `halogen`。
 */
export function backendLabel(id: string): string {
  if (id.startsWith("halogen")) return "halogen";
  return id;
}

/**
 * halogen 后端是不是「不许删」的那台——与后端 `engine::is_protected_halogen` 同口径：
 * 速率标定（按 halogen 机位取原生速率）与 halogen/gufo 控制台都以它为锚点。
 * 引擎为主、名字兜底（引擎列可能被清空，`halogen`/`hal` 是 halctl 认的机位别名）。
 */
export function isProtectedHalogen(b: { name: string; engine?: string | null }): boolean {
  const norm = (s?: string | null) => (s ?? "").trim().toLowerCase();
  const e = norm(b.engine);
  return e === "halogen" || e === "hal" || norm(b.name) === "halogen" || norm(b.name) === "hal";
}
