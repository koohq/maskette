import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, waitFor } from "@testing-library/react";
import { forwardRef } from "react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { writeAutoEyeSymmetry } from "./autoFixPreferences";
import { WorkspacePage } from "./WorkspacePage";

type Version = ReturnType<typeof version>;

const conversation = {
  id: "p1",
  title: "作品名A",
  created_at: "2026-11-16T00:00:00Z",
  locale: "ja",
  status: "idle",
  running: false,
  last_seq: 1,
  events: [],
  state: {
    references: [],
    images: [] as Version[],
    current_design_id: null as string | null,
    approved_design_id: null as string | null,
    current_front_id: null as string | null,
    approved_front_id: null as string | null,
  },
};

function version(id: string, role: string, source: string) {
  return {
    id,
    job_id: "job-1",
    instructions: "",
    parent_id: null as string | null,
    role,
    source,
    url: `/api/generated/${id}.png`,
    width: 800,
    height: 1100,
  };
}

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
const refresh = vi.fn(async () => undefined);
const saveManualVersion = vi.fn();
const correctEyeSymmetry = vi.fn();

vi.mock("../agent/useConversation", () => ({
  // A fresh conversation object per render, the way the real hook hands over a new fetch result.
  useConversation: () => ({
    conversation: { ...conversation, state: { ...conversation.state, images: [...conversation.state.images] } },
    events: [],
    loadError: null,
    refresh,
    running: false,
    setRunning: vi.fn(),
  }),
}));

vi.mock("../agent/agentApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../agent/agentApi")>()),
  saveManualVersion: (...args: unknown[]) => saveManualVersion(...args),
}));

vi.mock("./eyeSymmetryCorrection", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./eyeSymmetryCorrection")>()),
  correctEyeSymmetry: (...args: unknown[]) => correctEyeSymmetry(...args),
}));

vi.mock("./useEditorDrafts", () => ({
  useEditorDrafts: () => ({ draftIds: new Set<string>(), discard: vi.fn(), loaded: null, schedule: vi.fn() }),
}));

vi.mock("../editor/EditorWorkspace", () => ({
  EditorWorkspace: forwardRef(() => null),
}));

vi.mock("./AgentPanel", () => ({ AgentPanel: () => null }));

vi.mock("./ExplorerPanel", () => ({ ExplorerPanel: () => null }));

/** The workspace tree, re-created on purpose: each rerender must see the conversation's current images. */
function page() {
  return (
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/p/p1"]}>
        <Routes>
          <Route element={<WorkspacePage />} path="/p/:projectId" />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  );
}

/** Lets pending effects and promises run before asserting that nothing happened. */
function settle() {
  return new Promise((resolve) => setTimeout(resolve, 20));
}

describe("WorkspacePage automatic eye symmetry", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("ja");
  });

  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    conversation.state.images = [];
    conversation.state.current_design_id = null;
    conversation.state.current_front_id = null;
  });

  it("corrects a freshly generated uneven front view and saves it as its own version", async () => {
    conversation.state.images = [version("design-1", "design", "agent")];
    conversation.state.current_design_id = "design-1";
    const corrected = new Blob(["corrected"], { type: "image/png" });
    correctEyeSymmetry.mockResolvedValue({ blob: corrected, height: 1100, plan: {}, width: 800 });
    saveManualVersion.mockImplementation(async () => {
      conversation.state.images = [...conversation.state.images, version("front-2", "front", "auto_symmetry")];
      return conversation;
    });

    const { rerender } = render(page());
    // A version that already existed when the project opened is never corrected.
    await settle();
    expect(correctEyeSymmetry).not.toHaveBeenCalled();

    conversation.state.images = [...conversation.state.images, version("front-1", "front", "agent")];
    rerender(page());

    await waitFor(() => expect(saveManualVersion).toHaveBeenCalledTimes(1));
    expect(correctEyeSymmetry).toHaveBeenCalledWith({
      imageUrl: "/api/agent/conversations/p1/versions/front-1/source",
      role: "front",
    });
    expect(saveManualVersion).toHaveBeenCalledWith("p1", {
      baseImageId: "front-1",
      image: corrected,
      note: i18n.t("workspace.autoSymmetryNote"),
      recipe: {},
      source: "auto_symmetry",
    });
    await waitFor(() => expect(refresh).toHaveBeenCalled());

    // The corrected version itself is never corrected again.
    rerender(page());
    await settle();
    expect(correctEyeSymmetry).toHaveBeenCalledTimes(1);
  });

  it("leaves a version whose eyes are already symmetric alone", async () => {
    conversation.state.images = [version("design-1", "design", "agent")];
    correctEyeSymmetry.mockResolvedValue(null);

    const { rerender } = render(page());
    conversation.state.images = [...conversation.state.images, version("front-1", "front", "agent")];
    rerender(page());

    await waitFor(() => expect(correctEyeSymmetry).toHaveBeenCalledTimes(1));
    await settle();
    expect(saveManualVersion).not.toHaveBeenCalled();
  });

  it("skips the user's own manual save but still corrects the next generation", async () => {
    conversation.state.images = [version("front-1", "front", "agent")];
    correctEyeSymmetry.mockResolvedValue(null);

    const { rerender } = render(page());
    conversation.state.images = [...conversation.state.images, version("front-2", "front", "manual")];
    rerender(page());
    await settle();
    expect(correctEyeSymmetry).not.toHaveBeenCalled();

    conversation.state.images = [...conversation.state.images, version("front-3", "front", "agent")];
    rerender(page());

    await waitFor(() => expect(correctEyeSymmetry).toHaveBeenCalledTimes(1));
    expect(correctEyeSymmetry).toHaveBeenCalledWith({
      imageUrl: "/api/agent/conversations/p1/versions/front-3/source",
      role: "front",
    });
  });

  it("does nothing when the user turned the correction off", async () => {
    writeAutoEyeSymmetry(false);
    conversation.state.images = [version("design-1", "design", "agent")];

    const { rerender } = render(page());
    conversation.state.images = [...conversation.state.images, version("front-1", "front", "agent")];
    rerender(page());
    await settle();

    expect(correctEyeSymmetry).not.toHaveBeenCalled();
  });
});
