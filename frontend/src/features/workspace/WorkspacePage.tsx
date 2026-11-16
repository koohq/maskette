import {
  IconArrowLeft,
  IconDeviceFloppy,
  IconDownload,
  IconMessageCircle,
  IconPhoto,
  IconSparkles,
} from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { type DragEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link, useParams } from "react-router-dom";
import { writeStored } from "../../app/storage";
import { ConfirmDialog } from "../../ui/IdeDialog";
import {
  AgentApiError,
  type ApproveAction,
  approveActionFor,
  type Conversation,
  cancelRun,
  type DesignImage,
  deleteAssistantMessage,
  isTurnaroundRole,
  prepareUpload,
  type ReferenceTag,
  regenerateLastTurn,
  renameConversation,
  saveManualVersion,
  sendMessage,
  startAnnotatedRevision,
  startLocalRevision,
  tagReference,
  uploadReferences,
  versionDownloadUrl,
  versionSourceUrl,
} from "../agent/agentApi";
import { apiErrorText, errorDetail } from "../agent/apiErrorText";
import { useConversation } from "../agent/useConversation";
import { type EditorTool, editorTools } from "../editor/components/EditorToolRail";
import {
  type EditorImageSavePayload,
  type EditorLocalGeneratePayload,
  type EditorRegeneratePayload,
  type EditorStatus,
  EditorWorkspace,
  type EditorWorkspaceHandle,
} from "../editor/EditorWorkspace";
import { LanguageSelector } from "../i18n/LanguageSelector";
import { SettingsLink } from "../settings/SettingsLink";
import { AgentPanel } from "./AgentPanel";
import {
  AGENT_OPEN_STORAGE_KEY,
  AGENT_WIDTH_STORAGE_KEY,
  readStoredAgentOpen,
  readStoredAgentWidth,
} from "./agentWidth";
import { readAutoEyeSymmetry } from "./autoFixPreferences";
import { ExplorerPanel } from "./ExplorerPanel";
import { correctEyeSymmetry, supportsEyeSymmetryCorrection } from "./eyeSymmetryCorrection";
import { ThemeToggle } from "./ThemeToggle";
import { useEditorDrafts } from "./useEditorDrafts";
import { AgentResizeHandle, EditorTab, ImageLightbox } from "./WorkspaceChrome";

type Activity = "explorer" | EditorTool;

const TURNAROUND_TOOLS: [EditorTool, ...EditorTool[]] = ["annotation", "liquify", "local-generate"];

export function WorkspacePage() {
  const { t } = useTranslation();
  const { projectId = null } = useParams();
  const queryClient = useQueryClient();
  const { conversation, events, running, setRunning, loadError, refresh } = useConversation(projectId);

  const [activity, setActivity] = useState<Activity>("explorer");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [agentOpen, setAgentOpen] = useState(readStoredAgentOpen);
  const [agentWidth, setAgentWidth] = useState(() => readStoredAgentWidth(window.innerWidth));
  const [editorTool, setEditorTool] = useState<EditorTool>("annotation");
  const [openTabs, setOpenTabs] = useState<string[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [editorStatus, setEditorStatus] = useState<EditorStatus | null>(null);
  const [panelEl, setPanelEl] = useState<HTMLDivElement | null>(null);
  const [toolbarEl, setToolbarEl] = useState<HTMLDivElement | null>(null);
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const [sendError, setSendError] = useState<string | null>(null);
  const [deletingReply, setDeletingReply] = useState<number | null>(null);
  const [lightboxUrl, setLightboxUrl] = useState<string | null>(null);
  const [toast, setToast] = useState<{ text: string; tone?: "error" } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [saving, setSaving] = useState(false);
  const [correctingEyes, setCorrectingEyes] = useState(false);
  const editorRef = useRef<EditorWorkspaceHandle | null>(null);
  const seenImageIds = useRef<Set<string> | null>(null);
  const symmetryCheckedIds = useRef<Set<string> | null>(null);
  const correctingEyesRef = useRef(false);

  const images = conversation?.state.images ?? [];
  const activeVersion = images.find((image) => image.id === activeTabId) ?? null;
  const dirty = Boolean(editorStatus?.dirty);
  const availableTools = isTurnaroundRole(activeVersion?.role) ? TURNAROUND_TOOLS : undefined;
  const drafts = useEditorDrafts(projectId, activeTabId);

  useEffect(() => writeStored(AGENT_OPEN_STORAGE_KEY, agentOpen ? "1" : "0"), [agentOpen]);
  useEffect(() => writeStored(AGENT_WIDTH_STORAGE_KEY, String(agentWidth)), [agentWidth]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 3200);
    return () => window.clearTimeout(timer);
  }, [toast]);

  // Reset per project.
  // biome-ignore lint/correctness/useExhaustiveDependencies: projectId is the trigger: reset the tabs when another project opens
  useEffect(() => {
    setOpenTabs([]);
    setActiveTabId(null);
    seenImageIds.current = null;
    symmetryCheckedIds.current = null;
    correctingEyesRef.current = false;
    setCorrectingEyes(false);
  }, [projectId]);

  const openVersion = useCallback((imageId: string) => {
    setOpenTabs((tabs) => (tabs.includes(imageId) ? tabs : [...tabs, imageId]));
    setActiveTabId(imageId);
  }, []);

  // Open the current version on load, then follow new versions as they arrive (unless the user is mid-edit).
  useEffect(() => {
    if (!conversation) return;
    const ids = conversation.state.images.map((image) => image.id);
    if (seenImageIds.current === null) {
      seenImageIds.current = new Set(ids);
      const initial =
        conversation.state.current_front_id ?? conversation.state.current_design_id ?? ids[ids.length - 1];
      if (initial) openVersion(initial);
      return;
    }
    const fresh = ids.filter((id) => !seenImageIds.current?.has(id));
    for (const id of fresh) seenImageIds.current?.add(id);
    const newest = fresh[fresh.length - 1];
    if (!newest) return;
    if (editorRef.current?.isDirty()) {
      setOpenTabs((tabs) => (tabs.includes(newest) ? tabs : [...tabs, newest]));
    } else {
      openVersion(newest);
    }
  }, [conversation, openVersion]);

  // Automatic eye symmetry. A freshly generated version can inherit an uneven eye pair from the references; the
  // correction is deterministic (landmarks + a mirrored eye patch), costs no generation, and is saved as its own
  // version, so the editor and the next stage both work on the corrected pixels and the original stays in the tree.
  useEffect(() => {
    if (!projectId || !conversation) return;
    const known = symmetryCheckedIds.current;
    if (known === null) {
      // Versions that already exist when the project opens are left alone.
      symmetryCheckedIds.current = new Set(conversation.state.images.map((image) => image.id));
      return;
    }
    const fresh = conversation.state.images.filter((image) => !known.has(image.id));
    if (fresh.length === 0) return;
    // Wait for the turn to settle, so the pass sees the finished stage output instead of a mid-turn one.
    if (running || correctingEyesRef.current) return;
    for (const image of fresh) known.add(image.id);
    // Only the newest version, and only when the app generated it: a manual save is the user's own decision, a
    // corrected version is symmetric already, and an older fresh version was superseded.
    const candidate = fresh[fresh.length - 1];
    if (candidate?.source !== "agent" || !supportsEyeSymmetryCorrection(candidate.role)) return;
    if (!readAutoEyeSymmetry()) return;

    correctingEyesRef.current = true;
    setCorrectingEyes(true);
    void (async () => {
      try {
        const correction = await correctEyeSymmetry({
          imageUrl: versionSourceUrl(projectId, candidate.id),
          role: candidate.role,
        });
        if (!correction) return;
        const updated = await saveManualVersion(projectId, {
          baseImageId: candidate.id,
          image: correction.blob,
          note: t("workspace.autoSymmetryNote"),
          recipe: {},
          source: "auto_symmetry",
        });
        const created = updated.state.images[updated.state.images.length - 1];
        if (created) known.add(created.id);
        await refresh();
        setToast({ text: t("workspace.autoSymmetryApplied") });
      } catch (error) {
        // A correction is a bonus, never a blocker: when it fails, the generated version stays as it is.
        console.warn("Automatic eye symmetry correction failed", error);
      } finally {
        correctingEyesRef.current = false;
        setCorrectingEyes(false);
      }
    })();
  }, [conversation, projectId, refresh, running, t]);

  useEffect(() => {
    // Keep the active tool valid for the open version (turnarounds only support a subset).
    if (availableTools && !availableTools.includes(editorTool)) setEditorTool(availableTools[0]);
  }, [availableTools, editorTool]);

  function closeTab(imageId: string) {
    setOpenTabs((tabs) => {
      const next = tabs.filter((id) => id !== imageId);
      if (imageId === activeTabId) setActiveTabId(next[next.length - 1] ?? null);
      return next;
    });
  }

  function selectActivity(next: Activity) {
    if (next === activity) {
      setSidebarOpen((open) => !open);
      return;
    }
    setActivity(next);
    setSidebarOpen(true);
    if (next !== "explorer") setEditorTool(next);
  }

  function reportError(error: unknown) {
    if (error instanceof AgentApiError && error.status === 409) {
      setToast({ text: t("workspace.busy"), tone: "error" });
      return;
    }
    setToast({ text: apiErrorText(t, error), tone: "error" });
  }

  async function sendChat(text: string, files: File[], extra: { action?: ApproveAction; imageId?: string } = {}) {
    if (!projectId) return false;
    setSendError(null);
    try {
      const prepared = await Promise.all(files.map((file) => prepareUpload(file)));
      setRunning(true);
      await sendMessage(projectId, { text, files: prepared, ...extra });
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
      return true;
    } catch (error) {
      setRunning(false);
      setSendError(apiErrorText(t, error));
      return false;
    }
  }

  function agentErrorText(error: unknown) {
    return apiErrorText(t, error);
  }

  async function regenerate() {
    if (!projectId) return;
    setSendError(null);
    try {
      setRunning(true);
      await regenerateLastTurn(projectId);
      await refresh();
    } catch (error) {
      setRunning(false);
      setSendError(agentErrorText(error));
    }
  }

  async function deleteReply(seq: number) {
    if (!projectId) return;
    await deleteAssistantMessage(projectId, seq); // errors are shown by the dialog
    setDeletingReply(null);
    await refresh();
  }

  async function addReferences(files: File[]) {
    if (!projectId) return;
    try {
      const prepared = await Promise.all(files.map((file) => prepareUpload(file)));
      await uploadReferences(projectId, prepared);
      await refresh();
    } catch (error) {
      reportError(error);
    }
  }

  async function retagReference(referenceId: string, kind: ReferenceTag) {
    if (!projectId) return;
    try {
      await tagReference(projectId, referenceId, kind);
      await refresh();
    } catch (error) {
      reportError(error);
    }
  }

  async function handleSave(payload: EditorImageSavePayload) {
    if (!projectId || !activeVersion) return;
    setSaving(true);
    try {
      const updated = await saveManualVersion(projectId, {
        baseImageId: activeVersion.id,
        image: payload.imageBlob,
        note: payload.annotationPrompt,
        recipe: payload.recipe,
      });
      const created = updated.state.images[updated.state.images.length - 1];
      if (!created) throw new Error("The saved version is missing from the project");
      seenImageIds.current?.add(created.id);
      drafts.discard(activeVersion.id);
      await refresh();
      openVersion(created.id);
      setToast({ text: t("workspace.savedVersion", { id: created.id }) });
    } catch (error) {
      reportError(error);
    } finally {
      setSaving(false);
    }
  }

  async function handleLocalGenerate(payload: EditorLocalGeneratePayload) {
    if (!projectId || !activeVersion) return;
    try {
      setRunning(true);
      await startLocalRevision(projectId, {
        baseImageId: activeVersion.id,
        baseImage: payload.baseImageBlob,
        mask: payload.maskImageBlob,
        editNote: payload.editNote,
        lockOutside: Boolean(payload.lockOutside),
        referenceIds: payload.selectedReferenceKeys,
        referenceFile: payload.uploadedReferences[0]?.file,
      });
      setAgentOpen(true);
    } catch (error) {
      setRunning(false);
      reportError(error);
    }
  }

  async function handleAnnotatedRegenerate(payload: EditorRegeneratePayload) {
    if (!projectId || !activeVersion) return;
    try {
      setRunning(true);
      await startAnnotatedRevision(projectId, {
        baseImageId: activeVersion.id,
        image: payload.annotatedImageBlob ?? payload.editedImageBlob,
        instructions: [payload.annotationPrompt, payload.promptNote].filter(Boolean).join("\n"),
      });
      setAgentOpen(true);
    } catch (error) {
      setRunning(false);
      reportError(error);
    }
  }

  async function saveCurrent() {
    if (!editorRef.current || !activeVersion || running || saving) return;
    await editorRef.current.save();
  }

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void saveCurrent();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  function onDrop(event: DragEvent) {
    event.preventDefault();
    setDragging(false);
    if (running) return;
    const files = [...event.dataTransfer.files].filter((file) => file.type.startsWith("image/"));
    if (!files.length) return;
    setAgentOpen(true);
    setPendingFiles((current) => [...current, ...files].slice(0, 6));
  }

  const localReferenceOptions = useMemo(
    () =>
      (conversation?.state.references ?? []).map((reference) => ({
        key: reference.id,
        label: reference.file_name,
        imageUrl: reference.url,
      })),
    [conversation?.state.references],
  );

  const runState: RunState = conversation?.status === "error" ? "error" : running ? "running" : "idle";

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: drop target for image files dragged from the OS onto the window; dropping has no keyboard form, and the composer's attach button adds the same files to the pending message
    <div
      className="ide-root"
      onDragLeave={(event) => {
        if (event.currentTarget === event.target) setDragging(false);
      }}
      onDragOver={(event) => {
        if ([...event.dataTransfer.types].includes("Files")) {
          event.preventDefault();
          setDragging(true);
        }
      }}
      onDrop={onDrop}
    >
      <TitleBar
        agentOpen={agentOpen}
        conversation={conversation}
        onRename={async (title) => {
          if (!projectId) return;
          await renameConversation(projectId, title);
          await refresh();
          void queryClient.invalidateQueries({ queryKey: ["projects"] });
        }}
        onOpenVersion={(id) => openVersion(id)}
        onToggleAgent={() => setAgentOpen((open) => !open)}
        runState={runState}
      />

      <div className="ide-body">
        <nav className="ide-activity">
          <ActivityItem
            active={activity === "explorer" && sidebarOpen}
            icon={<IconPhoto size={20} />}
            label={t("workspace.explorerShort")}
            title={t("workspace.explorer")}
            onClick={() => selectActivity("explorer")}
          />
          <div className="ide-activity-divider" />
          {editorTools.map((tool) => {
            const Icon = tool.icon;
            const enabled = Boolean(activeVersion) && (!availableTools || availableTools.includes(tool.key));
            return (
              <ActivityItem
                active={activity === tool.key && sidebarOpen}
                disabled={!enabled}
                icon={<Icon size={19} />}
                key={tool.key}
                label={t(`workspace.tools.${tool.key}`)}
                onClick={() => selectActivity(tool.key)}
              />
            );
          })}
        </nav>

        {sidebarOpen ? (
          <aside className="ide-sidebar">
            {activity === "explorer" ? (
              <ExplorerPanel
                activeVersionId={activeTabId}
                conversation={conversation}
                disabled={running}
                onAddReferences={(files) => void addReferences(files)}
                onOpenImage={setLightboxUrl}
                onOpenVersion={(id) => openVersion(id)}
                onTagReference={(referenceId, kind) => void retagReference(referenceId, kind)}
              />
            ) : (
              <>
                <div className="ide-sidebar-header">{t(`workspace.tools.${activity}`)}</div>
                <div className="ide-sidebar-scroll">
                  {activeVersion ? null : <p className="ide-empty">{t("workspace.openVersionFirst")}</p>}
                  {isTurnaroundRole(activeVersion?.role) ? (
                    <p className="ide-empty">{t("workspace.turnaroundTools")}</p>
                  ) : null}
                  <div ref={setPanelEl} />
                </div>
              </>
            )}
          </aside>
        ) : null}

        <section className="ide-editor">
          <div className="ide-tabs">
            {openTabs.map((id) => {
              const image = images.find((item) => item.id === id);
              if (!image) return null;
              const isActive = id === activeTabId;
              return (
                <EditorTab
                  active={isActive}
                  id={id}
                  key={id}
                  onClose={() => closeTab(id)}
                  onOpen={() => openVersion(id)}
                  roleLabel={t(`workspace.roles.${image.role}`)}
                  thumbnailUrl={image.url}
                  unsaved={(isActive && dirty) || drafts.draftIds.has(id)}
                />
              );
            })}
          </div>

          {activeVersion && projectId ? (
            <>
              <div className="ide-toolbar">
                <div className="ide-toolbar-view" ref={setToolbarEl} />
                <div className="ide-toolbar-actions">
                  {editorTool === "annotation" &&
                  (activeVersion.role === "front" || activeVersion.role === "design") ? (
                    <button
                      className="ide-button"
                      disabled={running}
                      onClick={() => void editorRef.current?.regenerate()}
                      title={t("workspace.regenerateFromAnnotations")}
                      type="button"
                    >
                      <IconSparkles size={14} />
                      {t("workspace.regenerateFromAnnotations")}
                    </button>
                  ) : null}
                  <a
                    className="ide-icon-button"
                    download
                    href={versionDownloadUrl(projectId, activeVersion.id)}
                    title={t("workspace.download")}
                  >
                    <IconDownload size={16} />
                  </a>
                  <button
                    className="ide-button ide-button-primary"
                    disabled={!dirty || running || saving}
                    onClick={() => void saveCurrent()}
                    title="Ctrl+S"
                    type="button"
                  >
                    <IconDeviceFloppy size={14} />
                    {saving ? t("workspace.saving") : t("workspace.saveVersion")}
                  </button>
                </div>
              </div>
              <div className="ide-canvas">
                {drafts.loaded ? (
                  <EditorWorkspace
                    activeTool={availableTools && !availableTools.includes(editorTool) ? availableTools[0] : editorTool}
                    availableTools={availableTools}
                    candidateIndex={1}
                    downloadOnSave={false}
                    imageCoordinateEditing
                    imageHeight={activeVersion.height}
                    imageUrl={versionSourceUrl(projectId, activeVersion.id)}
                    initialLandmarks={drafts.loaded.draft?.recipe.landmarks ?? null}
                    initialLocalEditNote={drafts.loaded.draft?.localEditNote}
                    initialMaskStrokes={drafts.loaded.draft?.maskStrokes}
                    onDraftChange={drafts.schedule}
                    recipe={drafts.loaded.draft?.recipe}
                    imageWidth={activeVersion.width}
                    isRegenerating={running}
                    key={activeVersion.id}
                    layout="ide"
                    localReferenceOptions={localReferenceOptions}
                    onActiveToolChange={setEditorTool}
                    onLocalGenerate={handleLocalGenerate}
                    onRegenerate={handleAnnotatedRegenerate}
                    onSave={handleSave}
                    onStatusChange={setEditorStatus}
                    panelTarget={activity !== "explorer" && sidebarOpen ? panelEl : null}
                    ref={editorRef}
                    showRegenerateActions={false}
                    toolbarTarget={toolbarEl}
                  />
                ) : null}
                <CanvasInfo activeVersion={activeVersion} correctingEyes={correctingEyes} editorStatus={editorStatus} />
              </div>
            </>
          ) : (
            <div className="ide-welcome">
              {loadError ? (
                <p className="agent-notice agent-notice-error">
                  {t("agent.errors.load", { message: errorDetail(t, loadError) })}
                </p>
              ) : (
                <>
                  <img alt="" className="h-12 w-12 opacity-60" src="/logo.png" />
                  <p className="max-w-sm text-sm">
                    {images.length ? t("workspace.openVersionFirst") : t("workspace.noVersions")}
                  </p>
                </>
              )}
            </div>
          )}
          {dragging ? <div className="ide-dropzone">{t("agent.dropHint")}</div> : null}
        </section>

        {agentOpen ? (
          <>
            <AgentResizeHandle onWidthChange={setAgentWidth} width={agentWidth} />
            <AgentPanel
              conversation={conversation}
              events={events}
              onApprove={(imageId) => {
                if (correctingEyesRef.current) {
                  // The corrected version has to exist before the stage is approved, or the next stage starts from
                  // the uneven eyes again.
                  setToast({ text: t("workspace.autoSymmetryBusy") });
                  return;
                }
                const action = approveActionFor(images.find((image) => image.id === imageId)?.role);
                if (action) void sendChat("", [], { action, imageId });
              }}
              onDeleteMessage={setDeletingReply}
              onFilesChange={setPendingFiles}
              onOpenImage={setLightboxUrl}
              onRegenerate={() => void regenerate()}
              onOpenVersion={(id) => openVersion(id)}
              onSend={(text, files) => sendChat(text, files)}
              onStop={() => projectId && void cancelRun(projectId)}
              pendingFiles={pendingFiles}
              running={running}
              sendError={sendError}
              width={agentWidth}
            />
          </>
        ) : null}
      </div>

      {lightboxUrl ? <ImageLightbox onClose={() => setLightboxUrl(null)} url={lightboxUrl} /> : null}
      {deletingReply !== null ? (
        <ConfirmDialog
          confirmLabel={t("agent.deleteMessage")}
          danger
          errorMessage={agentErrorText}
          message={t("agent.confirmDeleteMessage")}
          onCancel={() => setDeletingReply(null)}
          onConfirm={() => deleteReply(deletingReply)}
          title={t("agent.deleteMessage")}
        />
      ) : null}
      {toast ? (
        <div className="ide-toast" data-tone={toast.tone} role="status">
          {toast.text}
        </div>
      ) : null}
    </div>
  );
}

function ActivityItem({
  active,
  disabled = false,
  icon,
  label,
  title,
  onClick,
}: {
  active: boolean;
  disabled?: boolean;
  icon: React.ReactNode;
  label: string;
  title?: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={title ?? label}
      aria-pressed={active}
      className="ide-activity-item"
      data-active={active}
      disabled={disabled}
      onClick={onClick}
      title={title ?? label}
      type="button"
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

type StageState = "done" | "active" | "todo" | "skipped";

/** Where the project is in the two-stage flow: character design -> head shell front -> head shell four-view. */
export function projectStages(conversation: Conversation | null) {
  const state = conversation?.state;
  const images = state?.images ?? [];
  const latest = (roles: string[]) => [...images].reverse().find((image) => roles.includes(image.role))?.id ?? null;
  const designId = state?.approved_design_id ?? state?.current_design_id ?? latest(["design", "design_turnaround"]);
  const frontId = state?.approved_front_id ?? state?.current_front_id ?? latest(["front"]);
  const turnaroundId = latest(["turnaround"]);
  const designDone = Boolean(state?.approved_design_id);
  const frontDone = Boolean(state?.approved_front_id);
  const stage = (done: boolean, reached: boolean): StageState => (done ? "done" : reached ? "active" : "todo");
  return [
    {
      key: "design" as const,
      imageId: designId,
      // Head shell made straight from the references: the design stage was skipped.
      state: designDone ? "done" : !designId && frontId ? "skipped" : ("active" as StageState),
    },
    { key: "front" as const, imageId: frontId, state: stage(frontDone, designDone || Boolean(frontId)) },
    { key: "turnaround" as const, imageId: turnaroundId, state: turnaroundId ? "done" : stage(false, frontDone) },
  ];
}

function StageTrack({
  conversation,
  onOpenVersion,
}: {
  conversation: Conversation | null;
  onOpenVersion: (id: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <ol aria-label={t("workspace.stageHint")} className="ide-stages">
      {projectStages(conversation).map((stage, index) => (
        <li data-state={stage.state} key={stage.key}>
          {index > 0 ? <span aria-hidden className="ide-stage-arrow" /> : null}
          <button
            disabled={!stage.imageId}
            onClick={() => stage.imageId && onOpenVersion(stage.imageId)}
            title={t("workspace.stageHint")}
            type="button"
          >
            <span className="ide-stage-index">{stage.state === "done" ? "✓" : index + 1}</span>
            {t(`workspace.stages.${stage.key}`)}
          </button>
        </li>
      ))}
    </ol>
  );
}

/** Whether the assistant is working shows as a dot on its toggle (the chat says the rest), named in the label. */
type RunState = "idle" | "running" | "error";

const TOGGLE_AGENT_LABEL = {
  idle: "workspace.toggleAgent",
  running: "workspace.toggleAgentBusy",
  error: "workspace.toggleAgentError",
} as const satisfies Record<RunState, string>;

function TitleBar({
  conversation,
  agentOpen,
  runState,
  onToggleAgent,
  onRename,
  onOpenVersion,
}: {
  conversation: Conversation | null;
  agentOpen: boolean;
  runState: RunState;
  onToggleAgent: () => void;
  onRename: (title: string) => Promise<void>;
  onOpenVersion: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState("");

  useEffect(() => setDraft(conversation?.title ?? ""), [conversation?.title]);
  const toggleLabel = t(TOGGLE_AGENT_LABEL[runState]);

  function commit() {
    const title = draft.trim();
    if (title && title !== conversation?.title) void onRename(title);
    else setDraft(conversation?.title ?? "");
  }

  return (
    <header className="ide-titlebar">
      <Link aria-label={t("workspace.home")} className="ide-icon-button" title={t("workspace.home")} to="/">
        <IconArrowLeft size={16} />
      </Link>
      <img alt="Maskette" className="h-5 w-5" src="/logo-head.png" />
      <div className="ide-titlebar-title">
        <Link className="hover:text-(--ide-text)" to="/">
          {t("workspace.projects")}
        </Link>
        <span>/</span>
        {/* The hidden copy of the name in ::after sizes the field to its text, wide characters included. */}
        <span className="ide-titlebar-name" data-value={draft || t("workspace.untitled")}>
          <input
            aria-label={t("workspace.rename")}
            onBlur={commit}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") (event.target as HTMLInputElement).blur();
              if (event.key === "Escape") {
                setDraft(conversation?.title ?? "");
                (event.target as HTMLInputElement).blur();
              }
            }}
            placeholder={t("workspace.untitled")}
            value={draft}
          />
        </span>
      </div>
      <StageTrack conversation={conversation} onOpenVersion={onOpenVersion} />
      <button
        aria-label={toggleLabel}
        className="ide-icon-button ide-agent-toggle"
        data-active={agentOpen}
        onClick={onToggleAgent}
        title={toggleLabel}
        type="button"
      >
        <IconMessageCircle size={16} />
        {runState === "idle" ? null : <span aria-hidden className="ide-run-dot" data-state={runState} />}
      </button>
      <SettingsLink />
      <ThemeToggle />
      <LanguageSelector compact height={28} />
    </header>
  );
}

/** Which version is on the canvas and whether its face landmarks are ready, in a small label at the lower left. */
function CanvasInfo({
  activeVersion,
  correctingEyes,
  editorStatus,
}: {
  activeVersion: DesignImage;
  correctingEyes: boolean;
  editorStatus: EditorStatus | null;
}) {
  const { t } = useTranslation();
  const landmarks = editorStatus
    ? editorStatus.detectingLandmarks
      ? t("workspace.status.detecting")
      : editorStatus.landmarksReady
        ? t("workspace.status.landmarksReady")
        : t("workspace.status.noLandmarks")
    : null;
  const parts = [
    activeVersion.id,
    t(`workspace.roles.${activeVersion.role}`),
    `${activeVersion.width}×${activeVersion.height}`,
    t(`workspace.sources.${activeVersion.source}`),
    landmarks,
    correctingEyes ? t("workspace.status.correctingEyes") : null,
  ];
  return <div className="ide-canvas-info">{parts.filter(Boolean).join(" · ")}</div>;
}
