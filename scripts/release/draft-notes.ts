// Turn the commits of a release into a draft of its notes: the `feat:` commits under "New" and the `fix:`
// commits under "Fixes", in the order they were made. Other types (ci, chore, docs, test, refactor, ...) are
// internal work and are left out, as the commit conventions intend.

export type Commit = { subject: string; body: string };

type Entry = { kind: "feat" | "fix"; text: string };

// The marker the drafts use where a person has to write something; `pnpm release:publish` refuses notes that
// still contain it.
export const TODO_MARKER = "TODO:";

const conventional = /^(feat|fix)(?:\([^)]*\))?(!)?:\s*(.+)$/;
const reverted = /^Revert "(.+)"$/;

function entryOf(commit: Commit): Entry | undefined {
  const match = conventional.exec(commit.subject.trim());
  if (!match) return undefined;
  const [, kind, bang, rest] = match;
  const breaking = bang === "!" || /^BREAKING[ -]CHANGE:/m.test(commit.body);
  const text = (rest ?? "").trim();
  const sentence = text.charAt(0).toUpperCase() + text.slice(1);
  return { kind: kind === "feat" ? "feat" : "fix", text: breaking ? `Breaking: ${sentence}` : sentence };
}

// Commits come oldest first. A commit that git reverted later in the same range drops out with its revert.
export function draftNotes(commits: readonly Commit[]): string {
  const revertedSubjects = new Map<string, number>();
  for (const commit of commits) {
    const subject = reverted.exec(commit.subject.trim())?.[1];
    if (subject) revertedSubjects.set(subject, (revertedSubjects.get(subject) ?? 0) + 1);
  }

  const entries: Entry[] = [];
  for (const commit of commits) {
    const subject = commit.subject.trim();
    const pending = revertedSubjects.get(subject) ?? 0;
    if (pending > 0) {
      revertedSubjects.set(subject, pending - 1);
      continue;
    }
    const entry = entryOf(commit);
    if (entry) entries.push(entry);
  }

  const section = (title: string, kind: Entry["kind"]): string[] => {
    const lines = entries.filter((entry) => entry.kind === kind).map((entry) => `- ${entry.text}`);
    return lines.length ? [title, ...lines] : [];
  };
  const parts = [section("New", "feat"), section("Fixes", "fix")].filter((lines) => lines.length);
  if (!parts.length) {
    return `${TODO_MARKER} no feat or fix commits since the last release. Say what this release changes.\n`;
  }
  return `${parts.map((lines) => lines.join("\n")).join("\n\n")}\n`;
}

// The first release has no earlier tag, and listing every commit since the start would bury what matters.
export function firstReleaseNotes(version: string): string {
  return `${TODO_MARKER} this is the first release (${version}), so the commits are not listed. Say in a few lines what Maskette does, or just "First release."\n`;
}

// Notes that still hold a line a person was meant to write.
export function hasTodo(notes: string): boolean {
  return notes.split(/\r?\n/).some((line) => line.trimStart().startsWith(TODO_MARKER));
}
