import { IconDownload } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "../../ui/IdeDialog";
import { listConversations } from "../agent/agentApi";
import { apiErrorText } from "../agent/apiErrorText";
import { installUpdate, type Release } from "./updateApi";
import { UPDATE_STATUS_KEY, useUpdateStatus } from "./updates";

/**
 * The "Update" button for `release`, the dialog that confirms it (with the release notes), and how the install goes
 * afterwards. Once the installer starts, the app closes; the installer opens it again when it is done.
 */
export function UpdateAction({ release }: { release: Release }) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const status = useUpdateStatus();
  // The same list the projects page shows: closing the app would stop a generation in the middle.
  const projects = useQuery({ queryKey: ["projects"], queryFn: listConversations });
  const [confirming, setConfirming] = useState(false);
  const generating = projects.data?.some((project) => project.status === "running") ?? false;
  const install = status.data?.install;

  async function start() {
    queryClient.setQueryData(UPDATE_STATUS_KEY, await installUpdate(release.version));
    setConfirming(false);
  }

  if (install?.phase === "downloading") {
    const percent = install.total ? Math.min(100, Math.floor((install.downloaded / install.total) * 100)) : null;
    return (
      <span className="text-sm" role="status">
        {percent === null ? t("update.downloadingNoTotal") : t("update.downloading", { percent })}
      </span>
    );
  }
  if (install?.phase === "installing") {
    return (
      <span className="text-sm" role="status">
        {t("update.installing")}
      </span>
    );
  }

  const failed = install?.phase === "failed";
  return (
    <>
      {failed ? (
        <span className="text-xs text-(--ide-danger)" role="alert">
          {[t("update.failed"), t("agent.failures.detail", { detail: install.detail })].join(t("agent.failures.gap"))}
        </span>
      ) : null}
      <button className="ide-button ide-button-primary" onClick={() => setConfirming(true)} type="button">
        <IconDownload size={15} />
        {failed ? t("update.retry") : t("update.install")}
      </button>
      {confirming ? (
        <ConfirmDialog
          confirmLabel={t("update.install")}
          disabled={generating}
          errorMessage={(error) => apiErrorText(t, error)}
          message={t("update.confirmMessage")}
          onCancel={() => setConfirming(false)}
          onConfirm={start}
          title={t("update.confirmTitle", { version: release.version })}
        >
          {release.notes ? (
            <div>
              <h3 className="text-xs font-semibold">{t("update.notes")}</h3>
              <p className="ide-update-notes">{release.notes}</p>
            </div>
          ) : null}
          {release.published_at ? (
            <p className="text-xs text-(--ide-text-faint)">
              {t("update.published", {
                date: new Date(release.published_at * 1000).toLocaleDateString(i18n.language),
              })}
            </p>
          ) : null}
          {generating ? (
            <p className="text-sm text-(--ide-danger)" role="alert">
              {t("update.running")}
            </p>
          ) : null}
        </ConfirmDialog>
      ) : null}
    </>
  );
}
