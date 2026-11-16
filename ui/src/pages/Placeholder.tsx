import { PAGES, type PageKey } from "../lib/pages";

/** Milestone placeholder until the page's own slice lands (see DEVELOPMENT-PLAN). */
export function PlaceholderPage({ pageKey }: { pageKey: PageKey }) {
  const p = PAGES[pageKey];
  return (
    <div className="flex h-full items-center justify-center">
      <div className="max-w-sm rounded-lg border border-dashed border-slate-700 bg-slate-900/40 p-8 text-center">
        <p className="text-base font-medium text-ink-2">{p.title}</p>
        <p className="mt-4 text-xs text-ink-4">该页面的功能将在对应里程碑接入，当前为骨架占位。</p>
      </div>
    </div>
  );
}
