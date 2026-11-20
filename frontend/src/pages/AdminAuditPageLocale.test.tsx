import { ThemeProvider } from "@mui/material/styles";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { type AuditGenerationJob, getAuditGenerationJobs, getAuditSummary } from "../api/client";
import { kigTheme } from "../app/theme";
import i18n from "../i18n";
import { literalTranslations } from "../i18n/literalTranslations";
import ja from "../i18n/resources/ja";
import { AdminAuditPage } from "./AdminAuditPage";

vi.mock("../api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/client")>();
  return {
    ...actual,
    getAuditGenerationJobs: vi.fn(),
    getAuditSummary: vi.fn(),
  };
});

const HAN = /[㐀-鿿]/;
const KANA = /[぀-ヿ]/;

function collectStrings(value: unknown, out: Set<string>) {
  if (typeof value === "string") out.add(value.trim());
  else if (value && typeof value === "object") for (const child of Object.values(value)) collectStrings(child, out);
}

// Japanese UI text written only in kanji (成功率, 上限…) is fine as long as it is one of our translations.
const japaneseStrings = new Set<string>();
collectStrings(ja, japaneseStrings);
collectStrings(literalTranslations.ja, japaneseStrings);

// Text the backend or the user wrote (prompt notes, reference descriptions) is shown as is.
const userText = new Set(["眼神更柔和", "正面参考"]);

function visibleStrings(root: HTMLElement) {
  const strings: string[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent?.trim();
    if (text) strings.push(text);
  }
  for (const element of root.querySelectorAll("[aria-label], [placeholder], [title], [alt]")) {
    for (const attribute of ["aria-label", "placeholder", "title", "alt"]) {
      const text = element.getAttribute(attribute)?.trim();
      if (text) strings.push(text);
    }
  }
  return strings;
}

// Kanji-only Japanese built by the patterns in literalTranslations (numbers filled in).
const japaneseTemplates = [
  /^失敗率 [\d.]+%$/,
  /^累計 \d+ 回 · 成功率 [\d.]+%$/,
  /^上限 \d+$/,
  /^出力 \d+$/,
  /^出力 \d+ 枚 · 参考 \d+ 枚$/,
  /^全 \d+ 件$/,
  // The reference kind is a backend code such as "front".
  /^参考画像（[a-z_]+）$/,
];

function untranslated(strings: string[], locale: "en" | "ja") {
  return strings.filter((text) => {
    if (!HAN.test(text) || userText.has(text)) return false;
    if (locale === "en") return true;
    if (KANA.test(text)) return false;
    if (japaneseTemplates.some((template) => template.test(text))) return false;
    return !japaneseStrings.has(text);
  });
}

function job(overrides: Partial<AuditGenerationJob>): AuditGenerationJob {
  return {
    character_session_id: "session-a",
    created_at: "2026-07-02T00:00:00+00:00",
    generation_mode: "front_design",
    id: "job-a",
    outputs: [],
    phase_label: "",
    progress: 0,
    provider: "fixture",
    references: [],
    requirement_ids: [],
    status: "running",
    token_usage: null,
    updated_at: "2026-07-02T00:01:00+00:00",
    user_notes: "",
    user_requirements: "",
    ...overrides,
  };
}

function renderPage() {
  Object.defineProperty(window, "matchMedia", {
    value: vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
  vi.mocked(getAuditGenerationJobs).mockResolvedValue([
    job({
      id: "job-done",
      outputs: [
        {
          height: 1100,
          image_url: "/api/generated/session/job/outputs/candidate-1.webp",
          index: 1,
          object_key: "job/outputs/candidate-1.webp",
          width: 800,
        },
      ],
      progress: 100,
      references: [
        {
          description: "正面参考",
          image_url: "/api/audit/references/front.webp",
          kind: "front",
          reference_key: "front:front.webp",
        },
        {
          description: "",
          image_url: "/api/audit/references/plain.webp",
          kind: "",
          reference_key: "plain.webp",
        },
      ],
      status: "succeeded",
      user_notes: "眼神更柔和",
    }),
    job({ id: "job-revision", generation_mode: "front_revision", status: "failed" }),
    job({ id: "job-turnaround", generation_mode: "turnaround", status: "queued" }),
    job({ id: "job-cancelled", status: "cancelled" }),
    job({ id: "job-running" }),
  ]);
  vi.mocked(getAuditSummary).mockResolvedValue({
    active_users: 4,
    failure_rate: 0.0909,
    job_counts: { queued: 2, succeeded: 8, failed: 1 },
    parallel_slots_used: 2,
    queue_length: 2,
    quota_policy: { window_hours: 5, normal_window_limit: 8, premium_unlimited: false, parallel_generation_limit: 8 },
    success_rate: 0.7273,
    total_calls: 11,
    total_users: 5,
    token_usage: {
      cached_input_tokens: null,
      input_tokens: null,
      jobs_with_usage: 0,
      output_tokens: null,
      reasoning_output_tokens: null,
      total_tokens: null,
    },
    image_usage: {
      cached_input_tokens_per_image: 0,
      generated_images: 0,
      images_with_token_usage: 0,
      input_tokens_per_image: 0,
      jobs_with_outputs: 0,
      output_tokens_per_image: 0,
      reasoning_output_tokens_per_image: 0,
      total_tokens_per_image: 0,
    },
  });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <ThemeProvider theme={kigTheme}>
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <AdminAuditPage />
        </MemoryRouter>
      </QueryClientProvider>
    </ThemeProvider>,
  );
}

describe.each(["ja", "en"] as const)("AdminAuditPage in %s", (locale) => {
  beforeAll(async () => {
    await i18n.changeLanguage(locale);
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  afterAll(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  it("translates the dashboard, the job records and the usage limits", async () => {
    const { container } = renderPage();
    await screen.findByTestId("audit-generation-job-detail");
    await screen.findByTestId("audit-overview-charts");

    expect(untranslated(visibleStrings(container), locale)).toEqual([]);
    expect(container.textContent).not.toContain("KigBuddy");
    expect(container.textContent).toContain(locale === "ja" ? "Maskette 監査" : "Maskette Audit");
    if (locale === "ja") expect(container.textContent).not.toMatch(/配額|配额/);
  }, 15_000);
});
