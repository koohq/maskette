import { useTranslation } from "react-i18next";
import { usePalette, useThemeMode } from "../../app/providers";
import { DEFAULT_PALETTE, idePalettes, PALETTE_IDS } from "../../app/theme";

/** Picks the colour set of the whole app. Applies at once and is kept on this computer. */
export function PaletteSection() {
  const { t } = useTranslation();
  const { mode } = useThemeMode();
  const { palette, setPalette } = usePalette();
  const title = t("settings.appearanceSection");
  return (
    <section className="rounded-md border border-(--ide-border) bg-(--ide-panel) p-4">
      <h2 className="text-sm font-semibold">{title}</h2>
      <p className="mb-3 mt-1 text-xs text-(--ide-text-muted)">{t("settings.appearanceHint")}</p>
      <div aria-label={title} className="flex flex-col gap-1" role="radiogroup">
        {PALETTE_IDS.map((id) => {
          const colors = idePalettes[id][mode];
          const label = t(`settings.palettes.${id}`);
          return (
            <label
              className="flex cursor-pointer items-center gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-(--ide-hover)"
              key={id}
            >
              <input
                aria-label={label}
                checked={palette === id}
                name="palette"
                onChange={() => setPalette(id)}
                type="radio"
                value={id}
              />
              <span
                aria-hidden="true"
                className="flex h-5 w-9 items-center justify-center gap-1 rounded-sm"
                style={{ background: colors.background, border: `1px solid ${colors.borderStrong}` }}
              >
                <span className="h-2.5 w-2.5 rounded-full" style={{ background: colors.accent }} />
                <span className="h-2.5 w-2.5 rounded-full" style={{ background: colors.text }} />
              </span>
              <span className="flex-1">{label}</span>
              {id === DEFAULT_PALETTE ? (
                <span className="ide-badge" data-tone="accent">
                  {t("settings.paletteDefault")}
                </span>
              ) : null}
            </label>
          );
        })}
      </div>
    </section>
  );
}
