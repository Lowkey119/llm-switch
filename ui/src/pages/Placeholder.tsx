import { PAGES, type PageKey } from "../lib/pages";
import { t } from "../lib/i18n";

/** Milestone placeholder until the page's own slice lands (see DEVELOPMENT-PLAN). */
export function PlaceholderPage({ pageKey }: { pageKey: PageKey }) {
  const p = PAGES[pageKey];
  return (
    <div className="flex h-full items-center justify-center">
      <div className="max-w-sm rounded-lg border border-dashed border-slate-700 bg-slate-900/40 p-8 text-center">
        <p className="text-base font-medium text-ink-2">{t(p.titleKey)}</p>
        <p className="mt-4 text-sm text-ink-4">{t("ph.desc")}</p>
      </div>
    </div>
  );
}
