import { useState } from "react";
import { useTranslation } from "react-i18next";
import { readAutoEyeSymmetry, writeAutoEyeSymmetry } from "../workspace/autoFixPreferences";

/** Switches the automatic fixes run on generated versions. Applies at once and is kept on this computer. */
export function AutoFixSection() {
  const { t } = useTranslation();
  const [eyeSymmetry, setEyeSymmetry] = useState(readAutoEyeSymmetry);
  return (
    <section className="rounded-md border border-(--ide-border) bg-(--ide-panel) p-4">
      <h2 className="text-sm font-semibold">{t("settings.autoEyeSymmetrySection")}</h2>
      <label className="mt-3 flex cursor-pointer items-center gap-2 text-sm">
        <input
          checked={eyeSymmetry}
          onChange={(event) => {
            setEyeSymmetry(event.target.checked);
            writeAutoEyeSymmetry(event.target.checked);
          }}
          type="checkbox"
        />
        {t("settings.autoEyeSymmetryLabel")}
      </label>
      <p className="mt-1 text-xs text-(--ide-text-muted)">{t("settings.autoEyeSymmetryHint")}</p>
    </section>
  );
}
