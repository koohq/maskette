import { readFileSync } from "node:fs";
import type { Page, Route } from "@playwright/test";

/**
 * A stand-in for the Maskette backend: answers every `/api` request the screens make with fixed data, so a screen
 * looks the same on every run. A request nothing here answers fails the test, so a screen that starts calling a new
 * endpoint is noticed instead of being captured half loaded.
 */

// The app logo stands in for every image: generated versions, references and thumbnails.
const IMAGE = readFileSync(new URL("../public/logo.png", import.meta.url));

export const PROJECT_ID = "project-a";
export const EMPTY_PROJECT_ID = "project-empty";

const CREATED_AT = "2026-11-01T10:00:00+09:00";

const projects = [
  {
    id: PROJECT_ID,
    title: "キャラクターA 正面デザイン",
    created_at: CREATED_AT,
    status: "idle",
    thumbnail_url: "/api/files/thumb-a.png",
    version_count: 3,
  },
  {
    id: "project-b",
    title: "Character B with a rather long project name that keeps going past the card",
    created_at: "2026-10-28T18:30:00+09:00",
    status: "idle",
    thumbnail_url: "/api/files/thumb-b.png",
    version_count: 12,
  },
  {
    id: "project-c",
    title: "角色C",
    created_at: "2026-10-20T09:15:00+09:00",
    status: "running",
    thumbnail_url: null,
    version_count: 0,
  },
  {
    id: EMPTY_PROJECT_ID,
    title: "",
    created_at: "2026-10-01T12:00:00+09:00",
    status: "idle",
    thumbnail_url: null,
    version_count: 0,
  },
];

function image(id: string, role: string, source: string, parentId: string | null, instructions = "") {
  return {
    id,
    role,
    job_id: `job-${id}`,
    url: `/api/files/${id}.png`,
    width: 1024,
    height: 1024,
    instructions,
    parent_id: parentId,
    source,
  };
}

const images = [
  image("design-1", "design", "manual", null),
  image("design-2", "design", "manual", "design-1", "目を少し大きく、前髪を短く"),
  image("front-1", "front", "manual", "design-2"),
];

const events = [
  {
    seq: 1,
    type: "user_message",
    text: "この子のマスクを作りたいです。正面の参考画像と横顔を付けました。",
    attachments: [],
    created_at: CREATED_AT,
  },
  {
    seq: 2,
    type: "references_added",
    attachments: [
      { id: "ref-front", url: "/api/files/ref-front.png", kind: "front" },
      { id: "ref-side", url: "/api/files/ref-side.png", kind: "side" },
    ],
    created_at: CREATED_AT,
  },
  {
    seq: 3,
    type: "tool",
    tool: "analyze_references",
    status: "succeeded",
    progress: 100,
    created_at: CREATED_AT,
  },
  {
    seq: 4,
    type: "assistant_message",
    text: "参考画像を読みました。髪はオレンジのボブ、目は緑のつり目です。後ろ姿の参考がないので、後頭部は推測で描きます。",
    created_at: CREATED_AT,
  },
  { seq: 5, type: "tool", tool: "generate_design", status: "succeeded", progress: 100, created_at: CREATED_AT },
  {
    seq: 6,
    type: "image",
    role: "design",
    image_id: "design-1",
    url: "/api/files/design-1.png",
    width: 1024,
    height: 1024,
    source: "manual",
    created_at: CREATED_AT,
  },
  {
    seq: 7,
    type: "user_message",
    text: "目を少し大きく、前髪を短くしてください",
    attachments: [],
    created_at: CREATED_AT,
  },
  { seq: 8, type: "tool", tool: "revise_design", status: "succeeded", progress: 100, created_at: CREATED_AT },
  {
    seq: 9,
    type: "image",
    role: "design",
    image_id: "design-2",
    url: "/api/files/design-2.png",
    width: 1024,
    height: 1024,
    source: "manual",
    created_at: CREATED_AT,
  },
  {
    seq: 10,
    type: "tool",
    tool: "generate_front_view",
    status: "failed",
    progress: 40,
    error: "upstream_timeout",
    created_at: CREATED_AT,
  },
  { seq: 11, type: "notice", code: "step_limit", created_at: CREATED_AT },
  { seq: 12, type: "tool", tool: "generate_front_view", status: "succeeded", progress: 100, created_at: CREATED_AT },
  {
    seq: 13,
    type: "image",
    role: "front",
    image_id: "front-1",
    url: "/api/files/front-1.png",
    width: 1024,
    height: 1024,
    source: "manual",
    created_at: CREATED_AT,
  },
];

const conversation = {
  id: PROJECT_ID,
  title: projects[0]?.title ?? "",
  created_at: CREATED_AT,
  locale: "ja",
  status: "idle",
  running: false,
  last_seq: events.length,
  events,
  state: {
    references: [
      {
        id: "ref-front",
        key: "ref-front",
        kind: "front",
        url: "/api/files/ref-front.png",
        file_name: "front.png",
        note: "",
      },
      {
        id: "ref-side",
        key: "ref-side",
        kind: "side",
        url: "/api/files/ref-side.png",
        file_name: "side_profile_reference_with_a_long_file_name.png",
        note: "",
      },
    ],
    missing_views: ["back"],
    images,
    current_design_id: "design-2",
    approved_design_id: "design-2",
    current_front_id: "front-1",
    approved_front_id: null,
  },
};

const emptyConversation = {
  id: EMPTY_PROJECT_ID,
  title: "",
  created_at: CREATED_AT,
  locale: "ja",
  status: "idle",
  running: false,
  last_seq: 0,
  events: [],
  state: {
    references: [],
    missing_views: [],
    images: [],
    current_design_id: null,
    approved_design_id: null,
    current_front_id: null,
    approved_front_id: null,
  },
};

const settings = {
  writable: true,
  values: {
    agent_llm_provider: "openai_compatible",
    agent_llm_base_url: "https://llm.example.test/v1",
    agent_llm_model: "chat-model",
    agent_llm_extra_body: "",
    agent_claude_code_model: "",
    llm_provider: "openai_compatible",
    analysis_llm_base_url: "",
    analysis_llm_model: "",
    analysis_llm_extra_body: "",
    image_provider: "openrouter",
    siliconflow_image_model: "sf-model",
    ark_image_model: "ark-model",
    openrouter_base_url: "https://openrouter.example.test/api/v1",
    openrouter_image_model: "vendor/image-model",
    openrouter_extra_body: "",
  },
  overridden: ["image_provider"],
  options: {
    agent_llm_provider: ["openai_compatible", "claude_code"],
    llm_provider: ["openai_compatible", "codex", "claude_code"],
    image_provider: ["codex", "openrouter", "siliconflow", "ark"],
  },
  secrets: {
    agent_llm_api_key: { set: true, source: "override", hint: "…1234" },
    analysis_llm_api_key: { set: false, source: "", hint: "" },
    siliconflow_api_key: { set: false, source: "", hint: "" },
    ark_api_key: { set: false, source: "", hint: "" },
    openrouter_api_key: { set: true, source: "env", hint: "…5678" },
  },
  status: {
    agent_llm_key: true,
    analysis_llm_key: false,
    claude_cli: false,
    codex_cli: true,
    siliconflow_key: false,
    ark_key: false,
    openrouter_key: true,
  },
};

type Answer = { json: unknown } | { body: Buffer; contentType: string } | { sse: true };

function answer(method: string, path: string): Answer | null {
  if (method !== "GET") return null;
  if (path.startsWith("/api/files/")) return { body: IMAGE, contentType: "image/png" };
  if (path === "/api/agent/conversations") return { json: projects };
  if (path === "/api/settings") return { json: settings };

  const match = /^\/api\/agent\/conversations\/([^/]+)(\/.*)?$/.exec(path);
  if (!match) return null;
  const [, id, rest = ""] = match;
  const found = id === PROJECT_ID ? conversation : id === EMPTY_PROJECT_ID ? emptyConversation : null;
  if (!found) return null;
  if (rest === "") return { json: found };
  if (rest === "/events") return { sse: true };
  if (/^\/versions\/[^/]+\/source$/.test(rest)) return { body: IMAGE, contentType: "image/png" };
  return null;
}

/** Answers `/api` from the fixtures above; returns the requests nothing answered. */
export async function mockBackend(page: Page): Promise<string[]> {
  const unanswered: string[] = [];
  await page.route(
    (url) => url.pathname === "/api" || url.pathname.startsWith("/api/"),
    async (route: Route) => {
      const request = route.request();
      const path = new URL(request.url()).pathname;
      const reply = answer(request.method(), path);
      if (!reply) {
        unanswered.push(`${request.method()} ${path}`);
        await route.fulfill({ status: 404, json: { detail: "not_found" } });
      } else if ("json" in reply) {
        await route.fulfill({ json: reply.json });
      } else if ("sse" in reply) {
        // No live events; the long retry keeps EventSource from reconnecting while the screen is captured.
        await route.fulfill({ body: "retry: 3600000\n\n", contentType: "text/event-stream" });
      } else {
        await route.fulfill({ body: reply.body, contentType: reply.contentType });
      }
    },
  );
  return unanswered;
}
