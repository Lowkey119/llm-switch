// 「新增覆盖」行（计划 §6.1 自 ParamsCenter.tsx 拆出，行为不变）：
// 目录里挑键（按分组归堆）或自填键名，值 + 策略一起提交。
import { useMemo, useState } from "react";
import { Button, barSelectCls } from "../ui";
import { POLICIES } from "../../lib/params";
import { gotoConsole } from "../../lib/nav";
import { t, ts } from "../../lib/i18n";
import { GROUP_LABELS, type CatalogEntry } from "./types";

export function AddRow({
  onSave,
  saving,
  catalog,
  supported,
  gate,
  backendName,
}: {
  onSave: (v: { key: string; value: string; policy: string }) => void;
  saving: boolean;
  catalog: CatalogEntry[];
  supported: string[] | null;
  gate: boolean;
  backendName?: string;
}) {
  const first = catalog[0]?.key ?? "temperature";
  const [key, setKey] = useState(first);
  const [customKey, setCustomKey] = useState("");
  const [value, setValue] = useState("0.7");
  const [policy, setPolicy] = useState("auto");
  const groups = useMemo(() => {
    const m = new Map<string, CatalogEntry[]>();
    for (const en of catalog) {
      if (!m.has(en.group)) m.set(en.group, []);
      m.get(en.group)!.push(en);
    }
    return [...m.entries()];
  }, [catalog]);
  const entry = catalog.find((c) => c.key === key);
  const unsupported =
    gate && supported != null && entry && entry.group !== "runtime"
      ? !supported.includes(key)
      : false;
  const isRuntime = entry?.group === "runtime";
  return (
    <div className="space-y-2 p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <select
          value={key}
          onChange={(e) => setKey(e.target.value)}
          className={barSelectCls + " w-52 font-mono"}
        >
          {groups.map(([g, entries]) => (
            <optgroup key={g} label={t(GROUP_LABELS[g] ?? g)}>
              {entries.map((en) => {
                const off =
                  en.group === "runtime" ||
                  (gate && supported != null && !supported.includes(en.key));
                return (
                  <option key={en.key} value={en.key} disabled={off}>
                    {en.key}
                    {off ? t("pc.optUnsupported") : ""}
                  </option>
                );
              })}
            </optgroup>
          ))}
          <optgroup label={t("pc.escapeHatch")}>
            <option value="raw_override">{t("pc.rawOverrideOpt")}</option>
            <option value="__custom__">{t("pc.customOpt")}</option>
          </optgroup>
        </select>
        {key === "__custom__" ? (
          <input
            value={customKey}
            onChange={(e) => setCustomKey(e.target.value)}
            placeholder={t("pc.customPh")}
            className={barSelectCls + " w-44 font-mono"}
          />
        ) : null}
        <input
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={key === "raw_override" ? t("pc.valuePhRaw") : t("pc.valuePh")}
          className={barSelectCls + " w-72 font-mono"}
        />
        <select
          value={policy}
          onChange={(e) => setPolicy(e.target.value)}
          className={barSelectCls}
        >
          <option value="auto">
            {entry ? ts("pc.policyAutoDir", { p: entry.default_policy }) : t("pc.policyAutoFill")}
          </option>
          {POLICIES.map((p) => (
            <option key={p.v} value={p.v}>
              {t(p.labelKey)}
            </option>
          ))}
        </select>
        <Button
          variant="primary"
          disabled={
            saving ||
            (!key) ||
            unsupported ||
            isRuntime ||
            (key === "__custom__" && !customKey)
          }
          onClick={() =>
            onSave({
              key: key === "__custom__" ? customKey : key,
              value,
              policy,
            })
          }
        >
          {t("pc.addOverride")}
        </Button>
      </div>
      {isRuntime && (
        <div className="flex items-center gap-2 text-[var(--sem-busy-text)]">
          {t("pc.runtimeReadonly")}
          {backendName && (
            <button
              className="rounded border border-amber-500/40 px-1.5 py-0.5 text-2xs hover:bg-amber-500/10"
              onClick={() => gotoConsole(backendName)}
            >
              {t("pc.gotoConsole")}
            </button>
          )}
        </div>
      )}
      {unsupported && !isRuntime && (
        <div className="text-[var(--sem-busy-text)]">{t("pc.unsupportedKey")}</div>
      )}
    </div>
  );
}
