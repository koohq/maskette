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
  // Before the newest image, so the usual shots still end on its approve button; the workspace-design-notes
  // scene scrolls up to the card.
  { seq: 12, type: "tool", tool: "write_design_notes", status: "succeeded", progress: 100, created_at: CREATED_AT },
  {
    seq: 13,
    type: "design_notes",
    front_id: "front-1",
    language: "ja",
    views: ["front", "turnaround", "eyes"],
    sections: [
      {
        key: "eyes",
        text: "大きな丸い目です。虹彩は上が深い緑、下が明るい黄緑のグラデーションで、瞳孔は縦長です。ハイライトは上に大小 2 つ、下に小さな丸が 2 つです。",
      },
      { key: "brows", text: "細いオレンジブラウンのアーチ眉で、前髪のすき間から見えます。" },
      { key: "mouth", text: "口を閉じて口角を少し上げた笑顔です。" },
      { key: "skin", text: "明るい肌色で、両頬に淡いオレンジピンクのチークがあります。" },
      {
        key: "hair",
        text: "明るいオレンジの、肩までのウェーブボブです。前髪は眉にかかり、向かって右の側頭部から三つ編みが 1 本下がります。",
      },
      { key: "ears", text: "頭の上の左右に、髪と同じ色のふわふわした三角の猫耳があります。" },
      {
        key: "accessories",
        text: "向かって左の前髪に黄色い星のヘアクリップ、向かって右の三つ編みの先に緑の星の飾りがあります。",
      },
      { key: "requests", text: "星の髪飾りは 2 つとも取り外せるようにしてください。" },
    ],
    created_at: CREATED_AT,
  },
  { seq: 14, type: "tool", tool: "generate_front_view", status: "succeeded", progress: 100, created_at: CREATED_AT },
  {
    seq: 15,
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
  values: {
    agent_llm_provider: "openai_compatible",
    // The built-in defaults: OpenRouter for the chat, which analysis follows.
    agent_llm_base_url: "https://openrouter.ai/api/v1",
    agent_llm_model: "google/gemini-3.8-flash",
    agent_llm_extra_body: '{"reasoning": {"effort": "low"}}',
    agent_codex_model: "",
    agent_claude_code_model: "",
    llm_provider: "same_as_agent",
    analysis_llm_base_url: "",
    analysis_llm_model: "",
    analysis_llm_extra_body: "",
    image_provider: "openrouter",
    siliconflow_image_model: "sf-model",
    ark_image_model: "ark-model",
    openrouter_base_url: "https://openrouter.ai/api/v1",
    openrouter_image_model: "vendor/image-model",
    openrouter_extra_body: "",
  },
  overridden: ["image_provider"],
  options: {
    agent_llm_provider: ["openai_compatible", "codex", "claude_code"],
    llm_provider: ["same_as_agent", "openai_compatible", "codex", "claude_code"],
    image_provider: ["openrouter", "codex", "siliconflow", "ark"],
  },
  // One OpenRouter key for the chat, the analysis that follows it, and images.
  keys: { "openrouter.ai": { hint: "…1234" } },
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

/** OpenRouter's chat models (`GET /api/settings/models/agent`): a long id and every kind of price. */
const CHAT_MODELS = [
  {
    id: "google/gemini-3.8-flash",
    name: "Google: Gemini 3.8 Flash",
    price: { unit: "token", input: 0.3, output: 2.5 },
  },
  { id: "openai/gpt-6.2-astra", name: "OpenAI: GPT-6.2 Astra", price: { unit: "token", input: 4, output: 20 } },
  {
    id: "vendor/a-model-with-a-very-long-identifier-that-does-not-fit-in-the-list-preview-2026-11",
    name: "Vendor: Long",
    price: { unit: "token", input: 0.075, output: 0.3 },
  },
  { id: "vendor/free-model:free", name: "Vendor: Free", price: { unit: "token", input: 0, output: 0 } },
  { id: "openrouter/auto", name: "Auto Router", price: null },
];

/**
 * OpenRouter's candidates for the model fields (`GET /api/settings/models/{role}`). Analysis follows the chat here,
 * so the chat list offers only the models on both lists: every chat model but GPT-6.2 Astra, which takes no images,
 * so the list still shows the long id and every kind of price.
 */
const MODEL_CANDIDATES = {
  agent: CHAT_MODELS,
  analysis: CHAT_MODELS.filter((model) => model.id !== "openai/gpt-6.2-astra"),
  image: [
    {
      id: "openai/gpt-image-2.6-sunburst",
      name: "OpenAI: GPT Image 2.6 Sunburst",
      price: { unit: "token", input: null, output: 30 },
    },
    { id: "recraft/recraft-v4", name: "Recraft V4", price: { unit: "image", input: null, output: 0.04 } },
  ],
};

const licenseText = (id: string) => readFileSync(new URL(`../../LICENSES/${id}.txt`, import.meta.url), "utf8");

/**
 * The third-party notices `pnpm notices` writes into public/ (not committed, so a fresh checkout has none): a few of
 * each group, with the longest names and notes the real list has.
 */
const notices = {
  appVersion: "0.1.0",
  components: [
    {
      name: "ab_glyph",
      version: "0.2.32",
      group: "app",
      license: "Apache-2.0",
      url: "https://crates.io/crates/ab_glyph/0.2.32",
      texts: [1],
    },
    {
      name: "Rust standard library",
      version: "1.100.0",
      group: "app",
      license: "MIT",
      url: "https://github.com/rust-lang/rust",
      note: "Offered under MIT OR Apache-2.0; compiled into the app.",
      texts: [0],
    },
    {
      name: "unicode-ident",
      version: "1.0.27",
      group: "app",
      license: "MIT AND Unicode-3.0",
      url: "https://crates.io/crates/unicode-ident/1.0.27",
      texts: [0],
    },
    {
      name: "@mediapipe/tasks-vision",
      version: "0.10.35",
      group: "screen",
      license: "Apache-2.0",
      url: "https://www.npmjs.com/package/@mediapipe/tasks-vision/v/0.10.35",
      note: "Its WebAssembly module is built with third-party components; see the third_party folder of the MediaPipe source: https://github.com/google-ai-edge/mediapipe/tree/v0.10.35/third_party",
      texts: [1],
    },
    {
      name: "react",
      version: "19.3.1",
      group: "screen",
      license: "MIT",
      url: "https://www.npmjs.com/package/react/v/19.3.1",
      texts: [0],
    },
    {
      name: "Anime face landmark model (anime-face-detector, HRNetV2)",
      version: "0.0.1",
      group: "data",
      license: "MIT",
      url: "https://github.com/hysts/anime-face-detector",
      note: "anime-face-hrnetv2-int8.onnx: the weights of mmpose_anime-face_hrnetv2.pth from the v0.0.1 release, converted to ONNX and quantized to 8-bit integers (as found in Visual Studio Kigurumi).",
      texts: [0],
    },
    {
      name: "NSIS (Nullsoft Scriptable Install System)",
      group: "installer",
      license: "Zlib AND CPL-1.0",
      url: "https://nsis.sourceforge.io/",
      texts: [0],
    },
  ],
  texts: [licenseText("MIT"), licenseText("Apache-2.0")],
};

type Answer = { json: unknown } | { body: Buffer; contentType: string } | { sse: true };

function answer(method: string, path: string): Answer | null {
  if (method !== "GET") return null;
  if (path.startsWith("/api/files/")) return { body: IMAGE, contentType: "image/png" };
  if (path === "/api/agent/conversations") return { json: projects };
  if (path === "/api/settings") return { json: settings };
  const models = /^\/api\/settings\/models\/(agent|analysis|image)$/.exec(path);
  if (models) return { json: { models: MODEL_CANDIDATES[models[1] as keyof typeof MODEL_CANDIDATES] } };

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
  await page.route(
    (url) => url.pathname === "/third-party-notices.json",
    (route: Route) => route.fulfill({ json: notices }),
  );
  return unanswered;
}
