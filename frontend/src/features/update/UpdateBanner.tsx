import { IconX } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { UpdateAction } from "./UpdateAction";
import { dismissUpdate, useDismissedUpdate, useUpdateCheck, useUpdateStatus } from "./updates";

/**
 * Above the project list: a new version is out. Closing it hides that version until the next launch. Shown only on
 * the list, so it never gets in the way of work on a project.
 */
export function UpdateBanner() {
  const { t } = useTranslation();
  const check = useUpdateCheck();
  const status = useUpdateStatus();
  const dismissed = useDismissedUpdate();
  const release = check.data?.update;
  if (!release || dismissed === release.version) return null;

  const phase = status.data?.install.phase;
  const installing = phase === "downloading" || phase === "installing";
  return (
    <section aria-label={t("update.section")} className="ide-update-banner">
      <img alt="" className="ide-update-banner-mascot" src="/logo.png" />
      <p className="flex-1 text-sm font-semibold">{t("update.available", { version: release.version })}</p>
      <UpdateAction release={release} />
      {installing ? null : (
        <button
          aria-label={t("update.dismiss")}
          className="ide-icon-button"
          onClick={() => dismissUpdate(release.version)}
          title={t("update.dismiss")}
          type="button"
        >
          <IconX size={15} />
        </button>
      )}
    </section>
  );
}
