import { IconAlertTriangle, IconCircleCheck, IconCircleX, IconPlugConnected } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "../../ui/IdeDialog";
import { errorDetail } from "../agent/apiErrorText";
import { LanguageSelector } from "../i18n/LanguageSelector";
import { HomeLink } from "../workspace/HomeLink";
import { ThemeToggle } from "../workspace/ThemeToggle";
import { AutoFixSection } from "./AutoFixSection";
import { ModelField } from "./ModelField";
import { PaletteSection } from "./PaletteSection";
import {
  analysisBackend,
  type BackendSettings,
  backendRequirement,
  getBackendSettings,
  isRecommendedBackend,
  matchingPreset,
  normalizeUrl,
  type PresetMatch,
  type ProbeResult,
  type ProbeRole,
  probeBackend,
  regionUrl,
  resetBackendSettings,
  SAME_AS_AGENT,
  URL_PRESETS,
  URL_REGIONS,
  type UrlPreset,
  updateBackendSettings,
  urlPreset,
  usesSubscriptionCli,
} from "./settingsApi";

const ARK = urlPreset("ark");
const SILICONFLOW = urlPreset("siliconflow");

const settingsQueryKey = ["backend-settings"];

type Notice = { tone: "success" | "error"; text: string } | null;

/** Analysis reuses the chat assistant's extra fields only while it talks to the same URL (empty = same). */
function sharesAgentUrl(analysisUrl: string | undefined, agentUrl: string | undefined) {
  const analysis = normalizeUrl(analysisUrl);
  return analysis === "" || analysis === normalizeUrl(agentUrl);
}

export function SettingsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: settingsQueryKey, queryFn: getBackendSettings });
  const [form, setForm] = useState<Record<string, string>>({});
  // New API keys typed by the user; null clears the key saved on this page.
  const [secrets, setSecrets] = useState<Record<string, string | null>>({});
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);
  const [confirmingReset, setConfirmingReset] = useState(false);

  useEffect(() => {
    if (settings.data) {
      setForm(settings.data.values);
      setSecrets({});
    }
  }, [settings.data]);

  const data = settings.data;
  const changed = data ? Object.keys(form).filter((key) => form[key] !== data.values[key]) : [];
  const changedSecrets = Object.entries(secrets).filter(([, value]) => value === null || value.trim() !== "");
  const unsaved = changed.length + changedSecrets.length > 0;
  const disabled = busy || !data;

  async function apply(action: () => Promise<BackendSettings>) {
    setBusy(true);
    setNotice(null);
    try {
      queryClient.setQueryData(settingsQueryKey, await action());
      setNotice({ tone: "success", text: t("settings.saved") });
    } catch (error) {
      setNotice({
        tone: "error",
        text: t("settings.saveFailed", { error: errorDetail(t, error) }),
      });
    } finally {
      setBusy(false);
    }
  }

  function save() {
    void apply(() =>
      updateBackendSettings({
        ...Object.fromEntries(changed.map((key) => [key, (form[key] ?? "").trim()])),
        ...Object.fromEntries(changedSecrets.map(([key, value]) => [key, value === null ? null : value.trim()])),
      }),
    );
  }

  function reset() {
    setConfirmingReset(false);
    void apply(resetBackendSettings);
  }

  const set = (key: string) => (value: string) => setForm((current) => ({ ...current, [key]: value }));
  const secretField = (key: string, hint?: string) =>
    data ? (
      <SecretField
        disabled={disabled}
        hint={hint}
        onChange={(value) => setSecrets((current) => ({ ...current, [key]: value }))}
        pending={secrets[key]}
        saved={data.secrets[key]}
      />
    ) : null;

  const agentBackend = form["agent_llm_provider"];
  const followsAgent = form["llm_provider"] === SAME_AS_AGENT;

  /** Folds analysis onto the chat assistant's settings, or unfolds it onto a backend of its own. */
  function followAgent(follow: boolean) {
    if (follow) {
      set("llm_provider")(SAME_AS_AGENT);
      return;
    }
    // The choice saved before, else the backend analysis has been using.
    const saved = data?.values["llm_provider"];
    const offered = data?.options["llm_provider"] ?? [];
    let own = "openai_compatible";
    if (saved && saved !== SAME_AS_AGENT) own = saved;
    else if (agentBackend && offered.includes(agentBackend)) own = agentBackend;
    set("llm_provider")(own);
  }

  function followSummary() {
    const backend = t(`settings.providers.${agentBackend}`, { defaultValue: agentBackend ?? "" });
    const model = form["agent_llm_model"]?.trim();
    return agentBackend === "openai_compatible" && model
      ? t("settings.analysisFollowsModel", { backend, model })
      : t("settings.analysisFollows", { backend });
  }

  return (
    <div className="ide-home">
      <header className="ide-titlebar" style={{ height: 44 }}>
        <HomeLink />
        <span className="text-sm font-semibold">{t("settings.title")}</span>
        <span className="flex-1" />
        <ThemeToggle />
        <LanguageSelector compact />
      </header>
      <main className="ide-home-main" style={{ maxWidth: 760 }}>
        <h1 className="mb-2 text-xl font-semibold">{t("settings.title")}</h1>
        <p className="mb-1 text-sm text-(--ide-text-muted)">{t("settings.intro")}</p>
        <p className="mb-6 text-xs text-(--ide-text-faint)">{t("settings.keyStorageNote")}</p>
        {settings.isError ? <p className="text-sm text-(--ide-danger)">{t("settings.loadFailed")}</p> : null}
        {data ? (
          <div className="flex flex-col gap-4">
            <BackendSection
              data={data}
              disabled={disabled}
              hint={t("settings.agentHint")}
              onChange={set("agent_llm_provider")}
              settingKey="agent_llm_provider"
              title={t("settings.agentSection")}
              value={agentBackend}
            >
              {agentBackend === "openai_compatible" ? (
                <>
                  <UrlField
                    disabled={disabled}
                    label={t("settings.baseUrl")}
                    onChange={set("agent_llm_base_url")}
                    value={form["agent_llm_base_url"]}
                  />
                  <ModelField
                    baseUrl={form["agent_llm_base_url"] ?? ""}
                    disabled={disabled}
                    label={t("settings.model")}
                    onChange={set("agent_llm_model")}
                    modelRole="agent"
                    value={form["agent_llm_model"]}
                  />
                  {secretField("agent_llm_api_key", t("settings.agentKeyHint"))}
                  <TextField
                    disabled={disabled}
                    hint={t("settings.extraBodyHint")}
                    label={t("settings.extraBody")}
                    onChange={set("agent_llm_extra_body")}
                    placeholder="{}"
                    value={form["agent_llm_extra_body"]}
                  />
                  {data.values["agent_llm_provider"] === "openai_compatible" ? (
                    <ProbePanel probeRole="agent" unsaved={unsaved} />
                  ) : null}
                </>
              ) : null}
              {agentBackend === "codex" ? (
                <TextField
                  disabled={disabled}
                  label={t("settings.model")}
                  onChange={set("agent_codex_model")}
                  placeholder={t("settings.modelDefault")}
                  value={form["agent_codex_model"]}
                />
              ) : null}
              {agentBackend === "claude_code" ? (
                <TextField
                  disabled={disabled}
                  label={t("settings.model")}
                  onChange={set("agent_claude_code_model")}
                  placeholder={t("settings.modelDefault")}
                  value={form["agent_claude_code_model"]}
                />
              ) : null}
            </BackendSection>
            <BackendSection
              data={data}
              disabled={disabled}
              follow={{ checked: followsAgent, label: t("settings.analysisSameAsAgent"), onChange: followAgent }}
              hint={t("settings.analysisHint")}
              onChange={set("llm_provider")}
              settingKey="llm_provider"
              title={t("settings.analysisSection")}
              value={form["llm_provider"]}
            >
              {followsAgent ? (
                <>
                  <p className="text-xs text-(--ide-text-muted)">{followSummary()}</p>
                  {agentBackend === "openai_compatible" && analysisBackend(data.values) === "openai_compatible" ? (
                    <ProbePanel probeRole="analysis" unsaved={unsaved} />
                  ) : null}
                </>
              ) : null}
              {form["llm_provider"] === "openai_compatible" ? (
                <>
                  <UrlField
                    disabled={disabled}
                    label={t("settings.baseUrl")}
                    onChange={set("analysis_llm_base_url")}
                    placeholder={t("settings.sameAsAgent", { value: form["agent_llm_base_url"] })}
                    value={form["analysis_llm_base_url"]}
                  />
                  <ModelField
                    baseUrl={form["analysis_llm_base_url"]?.trim() || (form["agent_llm_base_url"] ?? "")}
                    disabled={disabled}
                    label={t("settings.model")}
                    onChange={set("analysis_llm_model")}
                    placeholder={t("settings.sameAsAgent", { value: form["agent_llm_model"] })}
                    modelRole="analysis"
                    value={form["analysis_llm_model"]}
                  />
                  {secretField("analysis_llm_api_key", t("settings.analysisKeyHint"))}
                  <TextField
                    disabled={disabled}
                    hint={t("settings.analysisExtraBodyHint")}
                    label={t("settings.extraBody")}
                    onChange={set("analysis_llm_extra_body")}
                    placeholder={
                      sharesAgentUrl(form["analysis_llm_base_url"], form["agent_llm_base_url"])
                        ? t("settings.sameAsAgent", { value: form["agent_llm_extra_body"] || "{}" })
                        : "{}"
                    }
                    value={form["analysis_llm_extra_body"]}
                  />
                  <p className="text-xs text-(--ide-text-muted)">{t("settings.visionRequired")}</p>
                  {analysisBackend(data.values) === "openai_compatible" ? (
                    <ProbePanel probeRole="analysis" unsaved={unsaved} />
                  ) : null}
                </>
              ) : null}
            </BackendSection>
            <BackendSection
              data={data}
              disabled={disabled}
              hint={t("settings.imageHint")}
              onChange={set("image_provider")}
              settingKey="image_provider"
              title={t("settings.imageSection")}
              value={form["image_provider"]}
            >
              {form["image_provider"] === "codex_bridge" ? (
                <p className="text-xs text-(--ide-danger)" role="alert">
                  {t("settings.codexBridgeRemoved")}
                </p>
              ) : null}
              {form["image_provider"] === "openrouter" ? (
                <>
                  <TextField
                    disabled={disabled}
                    label={t("settings.baseUrl")}
                    onChange={set("openrouter_base_url")}
                    value={form["openrouter_base_url"]}
                  />
                  <ModelField
                    baseUrl={form["openrouter_base_url"] ?? ""}
                    disabled={disabled}
                    hint={t("settings.openrouterModelHint")}
                    label={t("settings.model")}
                    onChange={set("openrouter_image_model")}
                    modelRole="image"
                    value={form["openrouter_image_model"]}
                  />
                  {secretField("openrouter_api_key", t("settings.openrouterKeyHint"))}
                  <TextField
                    disabled={disabled}
                    hint={t("settings.openrouterExtraBodyHint")}
                    label={t("settings.extraBody")}
                    onChange={set("openrouter_extra_body")}
                    placeholder="{}"
                    value={form["openrouter_extra_body"]}
                  />
                </>
              ) : null}
              {form["image_provider"] === "siliconflow" ? (
                <>
                  <UrlField
                    disabled={disabled}
                    label={t("settings.baseUrl")}
                    onChange={set("siliconflow_base_url")}
                    service={SILICONFLOW}
                    value={form["siliconflow_base_url"]}
                  />
                  <TextField
                    disabled={disabled}
                    label={t("settings.model")}
                    onChange={set("siliconflow_image_model")}
                    value={form["siliconflow_image_model"]}
                  />
                  {secretField("siliconflow_api_key")}
                </>
              ) : null}
              {form["image_provider"] === "ark" ? (
                <>
                  <UrlField
                    disabled={disabled}
                    label={t("settings.baseUrl")}
                    onChange={set("ark_base_url")}
                    service={ARK}
                    value={form["ark_base_url"]}
                  />
                  <TextField
                    disabled={disabled}
                    label={t("settings.model")}
                    onChange={set("ark_image_model")}
                    value={form["ark_image_model"]}
                  />
                  {secretField("ark_api_key")}
                </>
              ) : null}
            </BackendSection>
            <div className="flex items-center gap-3">
              <button
                className="ide-button ide-button-primary"
                disabled={disabled || changed.length + changedSecrets.length === 0}
                onClick={save}
                type="button"
              >
                {t("settings.save")}
              </button>
              <button
                className="ide-button"
                disabled={disabled || data.overridden.length === 0}
                onClick={() => setConfirmingReset(true)}
                type="button"
              >
                {t("settings.reset")}
              </button>
              {notice ? (
                <span
                  className="text-sm"
                  role="status"
                  style={{ color: notice.tone === "error" ? "var(--ide-danger)" : "var(--ide-success)" }}
                >
                  {notice.text}
                </span>
              ) : null}
            </div>
          </div>
        ) : null}
        <div className="mt-6">
          <AutoFixSection />
        </div>
        <div className="mt-6">
          <PaletteSection />
        </div>
      </main>
      {confirmingReset ? (
        <ConfirmDialog
          confirmLabel={t("settings.reset")}
          message={t("settings.confirmReset")}
          onCancel={() => setConfirmingReset(false)}
          onConfirm={reset}
          title={t("settings.reset")}
        />
      ) : null}
    </div>
  );
}

/** A choice's one-line note: the role's own when it has one (`settings.providerNotes.<setting>.<choice>`), else the shared one. */
function optionNote(t: ReturnType<typeof useTranslation>["t"], settingKey: string, option: string): string {
  return t([`settings.providerNotes.${settingKey}.${option}`, `settings.providerNotes.${option}`], {
    defaultValue: "",
  });
}

function BackendSection({
  title,
  hint,
  settingKey,
  value,
  data,
  disabled,
  onChange,
  follow,
  children,
}: {
  title: string;
  hint: string;
  settingKey: string;
  value: string | undefined;
  data: BackendSettings;
  disabled: boolean;
  onChange: (value: string) => void;
  /** A checkbox that folds the choices away and uses another section's settings instead. */
  follow?: { checked: boolean; label: string; onChange: (checked: boolean) => void };
  children?: ReactNode;
}) {
  const { t } = useTranslation();
  const options = (data.options[settingKey] ?? []).filter((option) => option !== SAME_AS_AGENT);
  const folded = follow?.checked === true;
  const selectedRequirement = value && !folded ? backendRequirement(settingKey, value) : undefined;
  return (
    <section className="rounded-md border border-(--ide-border) bg-(--ide-panel) p-4">
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-semibold">{title}</h2>
        {data.overridden.includes(settingKey) ? (
          <span className="ide-badge" data-tone="accent">
            {t("settings.overridden")}
          </span>
        ) : null}
      </div>
      <p className="mb-3 mt-1 text-xs text-(--ide-text-muted)">{hint}</p>
      {follow ? (
        <label className="mb-2 flex cursor-pointer items-center gap-2 text-sm">
          <input
            checked={follow.checked}
            disabled={disabled}
            onChange={(event) => follow.onChange(event.target.checked)}
            type="checkbox"
          />
          {follow.label}
        </label>
      ) : null}
      {folded ? null : (
        <div className="flex flex-col gap-1" role="radiogroup" aria-label={title}>
          {options.map((option) => {
            const requirement = backendRequirement(settingKey, option);
            const ready = !requirement || data.status[requirement] !== false;
            const label = t(`settings.providers.${option}`, { defaultValue: option });
            const note = optionNote(t, settingKey, option);
            const noteId = `${settingKey}-${option}-note`;
            return (
              <label
                className="flex cursor-pointer items-start gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-(--ide-hover)"
                key={option}
              >
                <input
                  aria-describedby={note ? noteId : undefined}
                  aria-label={label}
                  checked={value === option}
                  className="mt-1"
                  disabled={disabled}
                  name={settingKey}
                  onChange={() => onChange(option)}
                  type="radio"
                  value={option}
                />
                {/* The badges sit beside the name, so the note below has the whole width. */}
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2">
                    <span className="flex-1">{label}</span>
                    {isRecommendedBackend(settingKey, option) ? (
                      <span className="ide-badge" data-tone="accent">
                        {t("settings.recommended")}
                      </span>
                    ) : null}
                    {requirement ? (
                      <span
                        className="ide-badge"
                        data-tone={ready ? "success" : undefined}
                        title={ready ? undefined : t(`settings.status.${requirement}`)}
                      >
                        {ready ? t("settings.ready") : t("settings.missing")}
                      </span>
                    ) : null}
                  </span>
                  {note ? (
                    <span className="block text-xs text-(--ide-text-muted)" id={noteId}>
                      {note}
                    </span>
                  ) : null}
                </span>
              </label>
            );
          })}
        </div>
      )}
      {selectedRequirement && data.status[selectedRequirement] === false ? (
        <p className="mt-2 text-xs text-(--ide-danger)">{t(`settings.status.${selectedRequirement}`)}</p>
      ) : null}
      {value && !folded && usesSubscriptionCli(value) ? (
        <p className="mt-2 flex items-start gap-1.5 text-xs text-(--ide-warning)" role="note">
          <IconAlertTriangle className="mt-px shrink-0" size={14} />
          <span>{t("settings.subscriptionCliNote")}</span>
        </p>
      ) : null}
      {children ? <div className="mt-3 flex flex-col gap-3">{children}</div> : null}
    </section>
  );
}

function TextField({
  label,
  value,
  placeholder,
  hint,
  disabled,
  onChange,
}: {
  label: string;
  value: string | undefined;
  placeholder?: string | undefined;
  hint?: string | undefined;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex flex-col gap-1 text-xs text-(--ide-text-muted)">
      {label}
      <input
        className="ide-input text-sm"
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        value={value ?? ""}
      />
      {hint ? <span>{hint}</span> : null}
    </label>
  );
}

/** The `settings.urlNotes` text for a matched URL: one per site for a service with two sites. */
function urlNoteKey({ preset, region }: PresetMatch) {
  return region ? `settings.urlNotes.${preset.id}.${region}` : `settings.urlNotes.${preset.id}`;
}

/**
 * An API URL, typed in or filled from the presets; the matching preset is shown pressed. A service with a mainland
 * China and an international site also gets a row to pick the site. `service` keeps the field to one service (the
 * image backends' URLs): then only that service's sites are offered.
 */
function UrlField({
  label,
  value,
  placeholder,
  disabled,
  onChange,
  service,
}: {
  label: string;
  value: string | undefined;
  placeholder?: string | undefined;
  disabled: boolean;
  onChange: (value: string) => void;
  service?: UrlPreset | undefined;
}) {
  const { t } = useTranslation();
  const matched = matchingPreset(value, service ? [service] : URL_PRESETS);
  const twoSites = service ?? matched?.preset;
  const note = matched ? t(urlNoteKey(matched), { defaultValue: "" }) : "";
  return (
    <div className="flex flex-col gap-1 text-xs text-(--ide-text-muted)">
      <label className="flex flex-col gap-1">
        {label}
        <input
          className="ide-input text-sm"
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          spellCheck={false}
          value={value ?? ""}
        />
      </label>
      {service ? null : (
        <fieldset aria-label={t("settings.urlPresets.label")} className="flex flex-wrap items-center gap-1.5">
          <span aria-hidden="true">{t("settings.urlPresets.label")}</span>
          {URL_PRESETS.map((preset) => {
            // Moving between two-site services keeps the site picked.
            const url = regionUrl(preset, matched?.region ?? "mainland");
            return (
              <button
                aria-pressed={matched?.preset.id === preset.id}
                className="ide-preset"
                disabled={disabled}
                key={preset.id}
                onClick={() => onChange(url)}
                title={url}
                type="button"
              >
                {t(`settings.urlPresets.${preset.id}`)}
              </button>
            );
          })}
        </fieldset>
      )}
      {twoSites?.international ? (
        <fieldset aria-label={t("settings.urlRegions.label")} className="flex flex-wrap items-center gap-1.5">
          <span aria-hidden="true">{t("settings.urlRegions.label")}</span>
          {URL_REGIONS.map((region) => {
            const url = regionUrl(twoSites, region);
            return (
              <button
                aria-pressed={matched?.region === region}
                className="ide-preset"
                disabled={disabled}
                key={region}
                onClick={() => onChange(url)}
                title={url}
                type="button"
              >
                {t(`settings.urlRegions.${region}`)}
              </button>
            );
          })}
        </fieldset>
      ) : null}
      {note ? <span>{note}</span> : null}
    </div>
  );
}

const probeIcons = {
  ok: <IconCircleCheck className="shrink-0 text-(--ide-success)" size={14} />,
  warn: <IconAlertTriangle className="shrink-0 text-(--ide-warning)" size={14} />,
  fail: <IconCircleX className="shrink-0 text-(--ide-danger)" size={14} />,
};

/** Tests the saved settings of one LLM role with a real request and lists what works. */
function ProbePanel({ probeRole, unsaved }: { probeRole: ProbeRole; unsaved: boolean }) {
  const { t } = useTranslation();
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<ProbeResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    setRunning(true);
    setError(null);
    setResult(null);
    try {
      setResult(await probeBackend(probeRole));
    } catch (caught) {
      setError(errorDetail(t, caught));
    } finally {
      setRunning(false);
    }
  }

  function describe(check: ProbeResult["checks"][number]) {
    if (check.id === "connection" && check.status === "fail") {
      const [reason, ...rest] = check.detail.split(": ");
      return t(`settings.probeReason.${reason}`, { defaultValue: check.detail, detail: rest.join(": ") });
    }
    return t(`settings.probeCheck.${check.id}.${check.status}`, { detail: check.detail });
  }

  return (
    <div className="flex flex-col gap-2" data-testid={`probe-${probeRole}`}>
      <div className="flex items-center gap-2">
        <button className="ide-button" disabled={running || unsaved} onClick={() => void run()} type="button">
          <IconPlugConnected size={14} />
          {running ? t("settings.probing") : t("settings.probe")}
        </button>
        <span className="text-xs text-(--ide-text-muted)">
          {unsaved ? t("settings.probeSaveFirst") : t("settings.probeHint")}
        </span>
      </div>
      {error ? <p className="text-xs text-(--ide-danger)">{error}</p> : null}
      {result ? (
        <div className="flex flex-col gap-1 rounded-md border border-(--ide-border) p-2 text-xs" role="status">
          <div className="font-semibold">
            {result.ok
              ? t("settings.probeOk", { model: result.model, ms: result.latency_ms ?? "-" })
              : t("settings.probeFailed", { model: result.model || "-" })}
          </div>
          {result.checks.map((check) => (
            <div className="flex items-start gap-1.5" key={check.id}>
              {probeIcons[check.status]}
              <span className="break-all">{describe(check)}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function SecretField({
  saved,
  pending,
  hint,
  disabled,
  onChange,
}: {
  saved: BackendSettings["secrets"][string] | undefined;
  pending: string | null | undefined;
  hint?: string | undefined;
  disabled: boolean;
  onChange: (value: string | null) => void;
}) {
  const { t } = useTranslation();
  const clearing = pending === null;
  const state = clearing
    ? t("settings.keyClearing")
    : saved?.set
      ? t("settings.keyFromPage", { hint: saved.hint })
      : t("settings.keyMissing");
  return (
    <div className="flex flex-col gap-1 text-xs text-(--ide-text-muted)">
      <label className="flex flex-col gap-1">
        <span>
          {t("settings.apiKey")} · {state}
        </span>
        <input
          autoComplete="off"
          className="ide-input text-sm"
          disabled={disabled || clearing}
          onChange={(event) => onChange(event.target.value)}
          placeholder={saved?.set ? t("settings.keyReplace") : t("settings.keyEnter")}
          spellCheck={false}
          type="password"
          value={pending ?? ""}
        />
      </label>
      {hint ? <span>{hint}</span> : null}
      {saved?.source === "override" || clearing ? (
        <button
          className="self-start text-(--ide-accent) hover:underline disabled:opacity-50"
          disabled={disabled}
          onClick={() => onChange(clearing ? "" : null)}
          type="button"
        >
          {clearing ? t("settings.keyUndoClear") : t("settings.keyClear")}
        </button>
      ) : null}
    </div>
  );
}
