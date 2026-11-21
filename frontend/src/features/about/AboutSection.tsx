import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { useUpdateStatus } from "../update/updates";
import { SOURCE_URL } from "./notices";

/**
 * The settings page's last part: what Maskette is licensed under, what it is based on, where its source is, and the
 * way to the licenses of the components it includes. The version shows where the backend tells it (the app's window).
 */
export function AboutSection() {
  const { t } = useTranslation();
  const status = useUpdateStatus();
  const version = status.data?.current_version;

  return (
    <section className="mt-6 rounded-md border border-(--ide-border) bg-(--ide-panel) p-4">
      <h2 className="text-sm font-semibold">{t("about.section")}</h2>
      {version ? <p className="mt-1 text-xs text-(--ide-text-muted)">{t("about.version", { version })}</p> : null}
      <p className="mt-3 text-sm">{t("about.license")}</p>
      <p className="mt-2 text-xs text-(--ide-text-muted)">{t("about.basedOn")}</p>
      <p className="mt-2 select-text break-words text-xs text-(--ide-text-muted)">
        {t("about.source", { url: SOURCE_URL })}
      </p>
      <div className="mt-3">
        <Link className="ide-button" to="/licenses">
          {t("about.thirdParty")}
        </Link>
      </div>
    </section>
  );
}
