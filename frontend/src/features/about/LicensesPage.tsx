import { IconChevronRight } from "@tabler/icons-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { LanguageSelector } from "../i18n/LanguageSelector";
import { HomeLink } from "../workspace/HomeLink";
import { ThemeToggle } from "../workspace/ThemeToggle";
import { NOTICE_GROUPS, NOTICES_TEXT_URL, type NoticeComponent, useThirdPartyNotices } from "./notices";

/**
 * The licenses of the components Maskette includes, opened from the settings page. One row per component; its
 * license texts appear when it is opened, so the hundreds of texts are not all on the page at once.
 */
export function LicensesPage() {
  const { t } = useTranslation();
  const notices = useThirdPartyNotices();
  const data = notices.data;

  return (
    <div className="ide-home">
      <header className="ide-titlebar" style={{ height: 44 }}>
        <HomeLink />
        <nav aria-label={t("licenses.breadcrumb")} className="flex min-w-0 items-center gap-1 text-sm">
          <Link className="text-(--ide-text-muted) hover:underline" to="/settings">
            {t("settings.title")}
          </Link>
          <span aria-hidden className="text-(--ide-text-faint)">
            /
          </span>
          <span className="truncate font-semibold">{t("licenses.title")}</span>
        </nav>
        <span className="flex-1" />
        <ThemeToggle />
        <LanguageSelector compact />
      </header>
      <main className="ide-home-main" style={{ maxWidth: 760 }}>
        <h1 className="mb-2 text-xl font-semibold">{t("licenses.title")}</h1>
        <p className="mb-4 text-sm text-(--ide-text-muted)">{t("licenses.intro")}</p>
        {notices.isPending ? <p className="text-sm text-(--ide-text-muted)">{t("licenses.loading")}</p> : null}
        {notices.isSuccess && !data ? <p className="text-sm text-(--ide-danger)">{t("licenses.unavailable")}</p> : null}
        {notices.isError ? <p className="text-sm text-(--ide-danger)">{t("licenses.unavailable")}</p> : null}
        {data ? (
          <>
            <a className="ide-button" download="third-party-notices.txt" href={NOTICES_TEXT_URL}>
              {t("licenses.saveText")}
            </a>
            {NOTICE_GROUPS.map((group) => {
              const members = data.components.filter((component) => component.group === group);
              if (members.length === 0) return null;
              return (
                <section className="mt-6" key={group}>
                  <h2 className="mb-2 text-sm font-semibold">
                    {t(`licenses.groups.${group}`)}
                    <span className="ml-2 font-normal text-(--ide-text-muted)">
                      {t("licenses.count", { count: members.length })}
                    </span>
                  </h2>
                  <ul className="rounded-md border border-(--ide-border) bg-(--ide-panel)">
                    {members.map((component) => (
                      <ComponentRow
                        component={component}
                        key={`${component.name}@${component.version ?? ""}`}
                        texts={data.texts}
                      />
                    ))}
                  </ul>
                </section>
              );
            })}
          </>
        ) : null}
      </main>
    </div>
  );
}

function ComponentRow({ component, texts }: { component: NoticeComponent; texts: string[] }) {
  const [open, setOpen] = useState(false);
  return (
    <li className="border-(--ide-border) border-b last:border-b-0">
      <details onToggle={(event) => setOpen(event.currentTarget.open)}>
        <summary className="flex cursor-pointer items-baseline gap-2 px-3 py-1.5 text-sm hover:bg-(--ide-hover)">
          <IconChevronRight
            aria-hidden
            className={`shrink-0 self-center text-(--ide-text-muted) transition-transform ${open ? "rotate-90" : ""}`}
            size={12}
          />
          <span className="min-w-0 break-words font-medium">{component.name}</span>
          {component.version ? (
            <span className="shrink-0 text-xs text-(--ide-text-muted)">{component.version}</span>
          ) : null}
          <span className="ml-auto shrink-0 text-xs text-(--ide-text-muted)">{component.license}</span>
        </summary>
        {open ? (
          <div className="px-3 pt-1 pb-3">
            {component.note ? <p className="mb-1 text-xs">{component.note}</p> : null}
            <p className="mb-2 select-text break-words text-xs text-(--ide-text-muted)">{component.url}</p>
            {component.texts.map((index) => (
              <pre
                className="mb-2 max-h-80 select-text overflow-auto whitespace-pre-wrap break-words rounded border border-(--ide-border) bg-(--ide-bg) p-2 text-xs"
                key={index}
              >
                {texts[index]}
              </pre>
            ))}
          </div>
        ) : null}
      </details>
    </li>
  );
}
