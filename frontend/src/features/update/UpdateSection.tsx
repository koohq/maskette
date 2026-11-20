import { useState } from "react";
import { useTranslation } from "react-i18next";
import { apiErrorText } from "../agent/apiErrorText";
import { UpdateAction } from "./UpdateAction";
import { readCheckOnLaunch, useUpdateCheck, useUpdateStatus, writeCheckOnLaunch } from "./updates";

/**
 * The settings page's part on updates: the running version, "Check now" and whether to check at startup. Shown only
 * where updates are supported (the installed app), so the dev build and a plain browser leave it out.
 */
export function UpdateSection() {
  const { t } = useTranslation();
  const status = useUpdateStatus();
  const check = useUpdateCheck();
  const [checkOnLaunch, setCheckOnLaunch] = useState(readCheckOnLaunch);
  if (!status.data?.supported) return null;

  const phase = status.data.install.phase;
  const installing = phase === "downloading" || phase === "installing";
  let result = null;
  if (check.isFetching) {
    result = null;
  } else if (check.isError) {
    result = (
      <span className="text-xs text-(--ide-danger)" role="alert">
        {apiErrorText(t, check.error)}
      </span>
    );
  } else if (check.data?.update) {
    result = (
      <>
        <span className="text-sm">{t("update.available", { version: check.data.update.version })}</span>
        <UpdateAction release={check.data.update} />
      </>
    );
  } else if (check.data) {
    result = (
      <span className="text-sm text-(--ide-success)" role="status">
        {t("update.upToDate")}
      </span>
    );
  }

  return (
    <section className="mt-6 rounded-md border border-(--ide-border) bg-(--ide-panel) p-4">
      <h2 className="text-sm font-semibold">{t("update.section")}</h2>
      <p className="mt-1 text-xs text-(--ide-text-muted)">
        {t("update.currentVersion", { version: status.data.current_version })}
      </p>
      <div className="mt-3 flex flex-wrap items-center gap-3">
        <button
          className="ide-button"
          disabled={check.isFetching || installing}
          onClick={() => void check.refetch()}
          type="button"
        >
          {check.isFetching ? t("update.checking") : t("update.checkNow")}
        </button>
        {result}
      </div>
      <label className="mt-4 flex cursor-pointer items-center gap-2 text-sm">
        <input
          checked={checkOnLaunch}
          onChange={(event) => {
            setCheckOnLaunch(event.target.checked);
            writeCheckOnLaunch(event.target.checked);
          }}
          type="checkbox"
        />
        {t("update.checkOnLaunch")}
      </label>
      <p className="mt-1 text-xs text-(--ide-text-muted)">{t("update.checkOnLaunchHint")}</p>
    </section>
  );
}
