import {
  IconDots,
  IconDownload,
  IconFolderPlus,
  IconPencil,
  IconPhoto,
  IconTrash,
  IconUpload,
} from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { inDesktopWindow } from "../../api/backend";
import { normalizeLocale } from "../../i18n/locales";
import { ConfirmDialog, PromptDialog } from "../../ui/IdeDialog";
import {
  type ConversationSummary,
  createConversation,
  deleteConversation,
  exportProjectUrl,
  importProject,
  listConversations,
  renameConversation,
} from "../agent/agentApi";
import { apiErrorText } from "../agent/apiErrorText";
import { LanguageSelector } from "../i18n/LanguageSelector";
import { SettingsLink } from "../settings/SettingsLink";
import { ThemeToggle } from "./ThemeToggle";

export function ProjectsPage() {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const projects = useQuery({ queryKey: ["projects"], queryFn: listConversations });
  const [creating, setCreating] = useState(false);
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<ConversationSummary | null>(null);
  const [renaming, setRenaming] = useState<ConversationSummary | null>(null);
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  async function create(name: string) {
    const project = await createConversation(normalizeLocale(i18n.language), name);
    await queryClient.invalidateQueries({ queryKey: ["projects"] });
    navigate(`/p/${project.id}`);
  }

  async function importProjectFile(file: File) {
    setImporting(true);
    setImportError(null);
    try {
      const project = await importProject(file);
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      navigate(`/p/${project.id}`);
    } catch (error) {
      setImportError(actionError(error));
    } finally {
      setImporting(false);
    }
  }

  async function remove(project: ConversationSummary) {
    await deleteConversation(project.id);
    setDeleting(null);
    await queryClient.invalidateQueries({ queryKey: ["projects"] });
  }

  async function rename(project: ConversationSummary, name: string) {
    await renameConversation(project.id, name);
    setRenaming(null);
    await queryClient.invalidateQueries({ queryKey: ["projects"] });
  }

  function actionError(error: unknown) {
    return apiErrorText(t, error);
  }

  return (
    <div className="ide-home">
      <header className="ide-titlebar" style={{ height: 44 }}>
        <img alt="Maskette" className="h-6 w-6" src="/logo.png" />
        {/* In the window the OS title bar already says Maskette; the logo's alt keeps the name for screen readers. */}
        {inDesktopWindow() ? null : <span className="text-sm font-semibold">Maskette</span>}
        <span className="flex-1" />
        <SettingsLink />
        <ThemeToggle />
        <LanguageSelector compact />
      </header>

      <main className="ide-home-main">
        <div className="mb-6 flex items-center gap-3">
          <h1 className="text-xl font-semibold">{t("workspace.projects")}</h1>
          <span className="flex-1" />
          <button
            className="ide-button"
            disabled={importing}
            onClick={() => fileInputRef.current?.click()}
            type="button"
          >
            <IconUpload size={15} />
            {importing ? t("workspace.importing") : t("workspace.importProject")}
          </button>
          <button className="ide-button ide-button-primary" onClick={() => setCreating(true)} type="button">
            <IconFolderPlus size={15} />
            {t("workspace.newProject")}
          </button>
          <input
            accept=".maskette,.vkp"
            className="hidden"
            onChange={(event) => {
              const file = event.target.files?.[0];
              event.target.value = "";
              if (file) void importProjectFile(file);
            }}
            ref={fileInputRef}
            type="file"
          />
        </div>

        {importError ? <p className="mb-4 text-xs text-(--ide-danger)">{importError}</p> : null}

        {projects.isError ? (
          <div className="ide-welcome" role="alert" style={{ padding: "12vh 0" }}>
            <p className="max-w-md">{t("workspace.loadProjectsFailed")}</p>
            <p className="max-w-md text-xs text-(--ide-danger)">{actionError(projects.error)}</p>
            <button
              className="ide-button"
              disabled={projects.isFetching}
              onClick={() => void projects.refetch()}
              type="button"
            >
              {t("agent.retry")}
            </button>
          </div>
        ) : projects.data?.length === 0 ? (
          <div className="ide-welcome" style={{ padding: "12vh 0" }}>
            <img alt="" className="h-14 w-14 opacity-80" src="/logo.png" />
            <p className="max-w-md">{t("workspace.emptyProjects")}</p>
            <button className="ide-button ide-button-primary" onClick={() => setCreating(true)} type="button">
              <IconFolderPlus size={15} />
              {t("workspace.newProject")}
            </button>
          </div>
        ) : (
          <div className="ide-project-grid">
            {projects.data?.map((project) => (
              <div className="ide-project-card" key={project.id}>
                <button className="block w-full text-left" onClick={() => navigate(`/p/${project.id}`)} type="button">
                  <div className="ide-project-thumb">
                    {project.thumbnail_url ? <img alt="" src={project.thumbnail_url} /> : <IconPhoto size={28} />}
                  </div>
                  <div className="ide-project-meta">
                    <div className="truncate text-sm font-semibold">{project.title || t("workspace.untitled")}</div>
                    <div className="mt-1 flex items-center gap-2 text-xs text-(--ide-text-muted)">
                      <span>{t("workspace.versionCount", { count: project.version_count })}</span>
                      <span>·</span>
                      <span>{new Date(project.created_at).toLocaleDateString(i18n.language)}</span>
                      {project.status === "running" ? (
                        <span className="ide-badge" data-tone="accent">
                          ●
                        </span>
                      ) : null}
                    </div>
                  </div>
                </button>
                <div className="ide-project-menu">
                  <button
                    aria-expanded={menuFor === project.id}
                    aria-label={t("workspace.projectActions")}
                    className="ide-icon-button"
                    onClick={() => setMenuFor(menuFor === project.id ? null : project.id)}
                    type="button"
                  >
                    <IconDots size={16} />
                  </button>
                  {menuFor === project.id ? (
                    <div className="absolute right-0 top-8 z-10 flex w-36 flex-col rounded-md border border-(--ide-border) bg-(--ide-panel-raised) p-1 shadow-lg">
                      <button
                        className="ide-tree-item"
                        onClick={() => {
                          setMenuFor(null);
                          setRenaming(project);
                        }}
                        type="button"
                      >
                        <IconPencil size={14} />
                        {t("workspace.rename")}
                      </button>
                      <a
                        className="ide-tree-item"
                        download
                        href={exportProjectUrl(project.id)}
                        onClick={() => setMenuFor(null)}
                      >
                        <IconDownload size={14} />
                        {t("workspace.exportProject")}
                      </a>
                      <button
                        className="ide-tree-item text-(--ide-danger)"
                        onClick={() => {
                          setMenuFor(null);
                          setDeleting(project);
                        }}
                        type="button"
                      >
                        <IconTrash size={14} />
                        {t("workspace.deleteProject")}
                      </button>
                    </div>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        )}
      </main>

      {creating ? (
        <PromptDialog
          allowEmpty
          confirmLabel={t("workspace.create")}
          errorMessage={actionError}
          maxLength={80}
          onCancel={() => setCreating(false)}
          onConfirm={create}
          placeholder={t("workspace.projectNamePlaceholder")}
          title={t("workspace.newProject")}
        />
      ) : null}
      {deleting ? (
        <ConfirmDialog
          confirmLabel={t("workspace.deleteProject")}
          danger
          errorMessage={actionError}
          message={t("workspace.confirmDeleteNamed", { name: deleting.title || t("workspace.untitled") })}
          onCancel={() => setDeleting(null)}
          onConfirm={() => remove(deleting)}
          title={t("workspace.deleteProject")}
        />
      ) : null}
      {renaming ? (
        <PromptDialog
          confirmLabel={t("common.save")}
          errorMessage={actionError}
          initialValue={renaming.title}
          maxLength={80}
          onCancel={() => setRenaming(null)}
          onConfirm={(name) => rename(renaming, name)}
          placeholder={t("workspace.projectNamePlaceholder")}
          title={t("workspace.rename")}
        />
      ) : null}
    </div>
  );
}
