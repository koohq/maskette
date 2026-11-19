import { IconChevronDown } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import type { TFunction } from "i18next";
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getModelCandidates, type ModelCandidate, type ModelPrice, type ModelRole, normalizeUrl } from "./settingsApi";

/** The backend keeps the lists for an hour too; a reload of the page asks it again. */
const LIST_STALE_MS = 60 * 60 * 1000;
/** A URL being typed asks for candidates once it rests this long. */
const URL_SETTLE_MS = 400;

function useSettled<T>(value: T, delay: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return settled;
}

/** The models the field offers at `baseUrl`: OpenRouter's that suit the role, or none (also when the list fails). */
function useModelCandidates(role: ModelRole, baseUrl: string): ModelCandidate[] {
  const url = useSettled(normalizeUrl(baseUrl), URL_SETTLE_MS);
  const candidates = useQuery({
    queryKey: ["model-candidates", role, url],
    queryFn: () => getModelCandidates(role, url),
    enabled: url !== "",
    staleTime: LIST_STALE_MS,
    retry: false,
  });
  return candidates.data ?? [];
}

/**
 * Whether `model` at `baseUrl` takes images, as OpenRouter's lists tell: `false` only for a listed chat model that
 * is missing from the models that take images; `undefined` when the lists cannot tell (another service, a list that
 * failed or is still loading, a name not listed).
 */
export function useTakesImages(model: string | undefined, baseUrl: string): boolean | undefined {
  const chatModels = useModelCandidates("agent", baseUrl);
  const imageModels = useModelCandidates("analysis", baseUrl);
  const name = model?.trim() ?? "";
  if (name === "" || imageModels.length === 0) return undefined;
  if (imageModels.some((candidate) => candidate.id === name)) return true;
  return chatModels.some((candidate) => candidate.id === name) ? false : undefined;
}

const dollars = new Intl.NumberFormat("en-US", { maximumSignificantDigits: 3 });

export function formatPrice(t: TFunction, price: ModelPrice): string {
  const usd = (value: number) => `$${dollars.format(value)}`;
  const { input, output } = price;
  if (output === null) return "";
  if (price.unit === "image") return t("settings.modelPrice.perImage", { price: usd(output) });
  if (input === null) return t("settings.modelPrice.perMillionTokens", { price: usd(output) });
  if (input === 0 && output === 0) return t("settings.modelPrice.free");
  return `${usd(input)} / ${usd(output)}`;
}

/**
 * A model name typed in, or picked from the role's candidates with their prices when the service lists them
 * (OpenRouter). Typing always works: a model missing from the list is still used as typed.
 */
export function ModelField({
  modelRole,
  alsoAnalysis,
  baseUrl,
  label,
  value,
  placeholder,
  hint,
  disabled,
  onChange,
}: {
  /** Which role's candidates to offer. */
  modelRole: ModelRole;
  /** Analysis uses this model too (it follows the chat assistant): offer only the candidates that also take images. */
  alsoAnalysis?: boolean | undefined;
  baseUrl: string;
  label: string;
  value: string | undefined;
  placeholder?: string | undefined;
  hint?: string | undefined;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const own = useModelCandidates(modelRole, baseUrl);
  // The same list as `own` (one request) unless analysis shares the model.
  const imageModels = useModelCandidates(alsoAnalysis ? "analysis" : modelRole, baseUrl);
  const candidates = useMemo(() => {
    if (!alsoAnalysis) return own;
    const takeImages = new Set(imageModels.map((model) => model.id));
    return own.filter((model) => takeImages.has(model.id));
  }, [alsoAnalysis, own, imageModels]);
  const id = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  // What was typed since the list opened; null shows every candidate.
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(-1);

  const shown = useMemo(() => {
    const needle = query?.trim().toLowerCase() ?? "";
    if (needle === "") return candidates;
    return candidates.filter(
      (model) => model.id.toLowerCase().includes(needle) || model.name.toLowerCase().includes(needle),
    );
  }, [candidates, query]);

  const listed = candidates.length > 0;
  const expanded = open && listed && !disabled;

  // The list opens below the field, which may sit at the bottom of the page.
  useEffect(() => {
    if (expanded) listRef.current?.parentElement?.scrollIntoView?.({ block: "nearest" });
  }, [expanded]);

  useEffect(() => {
    if (!expanded || active < 0) return;
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [expanded, active]);

  function showAll() {
    setOpen(true);
    setQuery(null);
    setActive(candidates.findIndex((model) => model.id === value?.trim()));
  }

  function choose(model: ModelCandidate) {
    onChange(model.id);
    setOpen(false);
    setQuery(null);
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!expanded) return showAll();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((current) => Math.min(Math.max(current + step, 0), shown.length - 1));
    } else if (event.key === "Enter" && expanded) {
      const model = shown[active];
      if (model) {
        event.preventDefault();
        choose(model);
      }
    } else if (event.key === "Escape" && expanded) {
      event.preventDefault();
      setOpen(false);
    }
  }

  const optionId = (index: number) => `${id}-option-${index}`;
  const inputProps = {
    autoComplete: "off",
    className: "ide-input text-sm",
    disabled,
    id: `${id}-input`,
    placeholder,
    spellCheck: false,
    value: value ?? "",
  };
  return (
    <div className="flex flex-col gap-1 text-xs text-(--ide-text-muted)">
      <label htmlFor={`${id}-input`}>{label}</label>
      {listed ? (
        <div className="relative">
          <input
            {...inputProps}
            aria-activedescendant={expanded && active >= 0 ? optionId(active) : undefined}
            aria-autocomplete="list"
            aria-controls={`${id}-list`}
            aria-expanded={expanded}
            onBlur={() => setOpen(false)}
            onChange={(event) => {
              onChange(event.target.value);
              setOpen(true);
              setQuery(event.target.value);
              setActive(-1);
            }}
            onKeyDown={onKeyDown}
            ref={inputRef}
            role="combobox"
            style={{ paddingRight: 30 }}
          />
          <button
            aria-label={t("settings.modelListShow")}
            className="ide-combobox-toggle"
            disabled={disabled}
            onClick={() => {
              if (expanded) {
                setOpen(false);
              } else {
                showAll();
                inputRef.current?.focus();
              }
            }}
            // Keeps the focus in the field, so the list stays open.
            onMouseDown={(event) => event.preventDefault()}
            tabIndex={-1}
            type="button"
          >
            <IconChevronDown size={14} />
          </button>
          {expanded ? (
            <div className="ide-combobox-popup">
              <p className="ide-combobox-note">
                {t(alsoAnalysis ? "settings.modelListNote.agentWithAnalysis" : `settings.modelListNote.${modelRole}`)}
              </p>
              <div
                aria-label={label}
                className="ide-combobox-list"
                id={`${id}-list`}
                // Clicks in the list keep the focus in the field.
                onMouseDown={(event) => event.preventDefault()}
                ref={listRef}
                role="listbox"
                tabIndex={-1}
              >
                {shown.map((model, index) => (
                  // The field takes the keys and points at the active option (aria-activedescendant).
                  // biome-ignore lint/a11y/useKeyWithClickEvents: the combobox field handles the keyboard.
                  <div
                    aria-selected={index === active}
                    className="ide-combobox-option"
                    data-index={index}
                    id={optionId(index)}
                    key={model.id}
                    onClick={() => choose(model)}
                    role="option"
                    tabIndex={-1}
                    title={model.name || undefined}
                  >
                    <span className="ide-combobox-id">{model.id}</span>
                    {model.price ? <span className="ide-combobox-price">{formatPrice(t, model.price)}</span> : null}
                  </div>
                ))}
              </div>
              {shown.length === 0 ? <p className="ide-combobox-note">{t("settings.modelNoMatch")}</p> : null}
            </div>
          ) : null}
        </div>
      ) : (
        <input {...inputProps} onChange={(event) => onChange(event.target.value)} />
      )}
      {hint ? <span>{hint}</span> : null}
    </div>
  );
}
