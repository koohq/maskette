import { IconCheck, IconCopy, IconDownload, IconFileText } from "@tabler/icons-react";
import type { TFunction } from "i18next";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SUPPORTED_LOCALES } from "../../i18n/locales";
import { downloadBlob, safeFileName } from "../../ui/downloadBlob";
import type { AgentEvent } from "./agentApi";

/** One part of the order notes, as the backend's `order_notes` event sends it. */
export type OrderNotesSection = { key: string; text: string };

/** The pictures the priority line can name, strongest first. */
const PRIORITY_VIEWS = ["front", "turnaround", "eyes"] as const;
const COPIED_FOR_MS = 2000;

function sectionsOf(event: AgentEvent): OrderNotesSection[] {
  const sections = Array.isArray(event["sections"]) ? (event["sections"] as OrderNotesSection[]) : [];
  return sections.filter((section) => typeof section.text === "string" && section.text.trim());
}

/**
 * Puts `text` on the clipboard. Where the page may not use the Clipboard API (some embedded browsers refuse it),
 * the old copy command of a selected text area still works while the click's user activation lasts.
 */
async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    try {
      return document.execCommand("copy");
    } catch {
      return false;
    } finally {
      area.remove();
    }
  }
}

/** The lines the app adds for the maker: which side is left, which picture wins, and how far to trust colours. */
function fixedLines(t: TFunction, views: string[]): string[] {
  const list = PRIORITY_VIEWS.filter((view) => views.includes(view))
    .map((view) => t(`agent.orderNotes.priorityParts.${view}`))
    .join(t("agent.orderNotes.listSeparator"));
  return [
    t("agent.orderNotes.sides"),
    ...(list ? [t("agent.orderNotes.priority", { list })] : []),
    t("agent.orderNotes.colors"),
  ];
}

/** The notes as plain text in `t`'s language, ready to paste into an order form. */
export function orderNotesText(t: TFunction, sections: OrderNotesSection[], views: string[]): string {
  const parts = sections.map((section) =>
    t("agent.orderNotes.sectionLine", {
      label: t(`agent.orderNotes.sections.${section.key}`, { defaultValue: section.key }),
      text: section.text.trim(),
    }),
  );
  const bullet = t("agent.orderNotes.bullet");
  return [...parts, "", ...fixedLines(t, views).map((line) => `${bullet}${line}`)].join("\n");
}

export function OrderNotesCard({
  event,
  approvedFrontId,
  title,
}: {
  event: AgentEvent;
  /** The front view approved now; notes for another one are out of date. */
  approvedFrontId: string | null | undefined;
  /** The project's name, for the saved file. */
  title: string;
}) {
  const { t, i18n } = useTranslation();
  const language = typeof event["language"] === "string" ? event["language"] : i18n.language;
  // The notes are for the maker, in their language: label them in it too.
  const notesT = i18n.getFixedT(language);
  const sections = sectionsOf(event);
  const views = Array.isArray(event["views"]) ? (event["views"] as string[]) : ["front"];
  const stale = Boolean(approvedFrontId) && event["front_id"] !== approvedFrontId;
  const languageLabel = SUPPORTED_LOCALES.find((locale) => locale.value === language)?.label ?? language;
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");

  useEffect(() => {
    if (copy === "idle") return;
    const timer = window.setTimeout(() => setCopy("idle"), COPIED_FOR_MS);
    return () => window.clearTimeout(timer);
  }, [copy]);

  const text = () => orderNotesText(notesT, sections, views);
  const copyText = async () => {
    setCopy((await copyToClipboard(text())) ? "copied" : "failed");
  };
  const save = () => {
    const name = t("agent.orderNotes.fileName");
    const fileName = safeFileName(title ? `${title}-${name}` : name, name);
    downloadBlob(new Blob([text()], { type: "text/plain;charset=utf-8" }), `${fileName}.txt`);
  };

  return (
    <li className="agent-image-card agent-notes-card ml-8">
      <div className="agent-notes-head">
        <IconFileText size={14} />
        <span className="font-semibold">{t("agent.orderNotes.title")}</span>
        <span className="ide-badge">{languageLabel}</span>
      </div>
      {stale ? <p className="agent-notes-stale">{t("agent.orderNotes.stale")}</p> : null}
      <dl className="agent-notes-body" lang={language}>
        {sections.map((section) => (
          <div className="agent-notes-section" key={section.key}>
            <dt>{notesT(`agent.orderNotes.sections.${section.key}`, { defaultValue: section.key })}</dt>
            <dd>{section.text.trim()}</dd>
          </div>
        ))}
      </dl>
      <ul className="agent-notes-fixed" lang={language}>
        {fixedLines(notesT, views).map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
      <p className="agent-notes-hint">{t("agent.orderNotes.hint")}</p>
      <p aria-live="polite" className="agent-notes-status" data-failed={copy === "failed" || undefined}>
        {copy === "copied" ? t("agent.orderNotes.copied") : copy === "failed" ? t("agent.orderNotes.copyFailed") : ""}
      </p>
      <div className="agent-image-card-footer">
        <span className="flex-1" />
        <button className="agent-button" onClick={() => void copyText()} type="button">
          {copy === "copied" ? <IconCheck size={14} /> : <IconCopy size={14} />}
          {t("agent.orderNotes.copy")}
        </button>
        <button className="agent-button" onClick={save} type="button">
          <IconDownload size={14} />
          {t("agent.orderNotes.save")}
        </button>
      </div>
    </li>
  );
}
