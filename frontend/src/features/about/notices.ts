import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import { publicUrl } from "../../ui/publicUrl";

/** Where Maskette's source code is published. */
export const SOURCE_URL = "https://github.com/koohq/maskette";

/** The license list as plain text, for saving (the same list the release attaches). */
export const NOTICES_TEXT_URL = publicUrl("/third-party-notices.txt");

/** The parts of Maskette the components go into, in the order the list shows them. */
export const NOTICE_GROUPS = ["app", "screen", "data", "installer"] as const;
export type NoticeGroup = (typeof NOTICE_GROUPS)[number];

const componentSchema = z.object({
  name: z.string(),
  version: z.string().optional(),
  group: z.enum(NOTICE_GROUPS),
  license: z.string(),
  url: z.string(),
  note: z.string().optional(),
  /** Indexes into `texts`. */
  texts: z.array(z.number().int().nonnegative()),
});

const noticesSchema = z.object({
  appVersion: z.string(),
  components: z.array(componentSchema),
  texts: z.array(z.string()),
});

export type NoticeComponent = z.output<typeof componentSchema>;
export type Notices = z.output<typeof noticesSchema>;

/**
 * The third-party notices `pnpm bundle` writes into the app (scripts/third-party-notices.ts), or null where there are
 * none: the dev server before `pnpm notices`, which answers a missing file with the app's own page.
 */
async function getNotices(): Promise<Notices | null> {
  const response = await fetch(publicUrl("/third-party-notices.json"));
  if (!response.ok) return null;
  try {
    const parsed = noticesSchema.safeParse(await response.json());
    return parsed.success ? parsed.data : null;
  } catch {
    return null;
  }
}

export function useThirdPartyNotices() {
  return useQuery({
    queryKey: ["third-party-notices"],
    queryFn: getNotices,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
    refetchOnWindowFocus: false,
  });
}
