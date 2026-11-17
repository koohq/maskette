import type { OverflowFinding } from "./overflow";

/**
 * Overflow the screens already have, so the tests fail only on new ones. Each entry matches findings by kind and by
 * the box they overflow (`box` from `findOverflow`), on the screens `screens` names (`<screen>-<locale>-<size>`).
 * A listed entry that a screen no longer shows fails that test too: remove or narrow it once the layout is fixed.
 */
export type KnownOverflow = {
  issue: string;
  screens: RegExp;
  kind: OverflowFinding["kind"];
  box: RegExp;
};

// Add an entry only for overflow that is accepted, saying why.
export const KNOWN_OVERFLOW: KnownOverflow[] = [
  {
    issue:
      "A project name can be 80 characters long; while it is being renamed it scrolls with the caret inside the " +
      "field, as typed text does, rather than the dialog growing to fit it.",
    screens: /^projects-rename-/,
    kind: "clipped",
    box: /form\.ide-dialog > input/,
  },
];

function matches(known: KnownOverflow, finding: OverflowFinding): boolean {
  return known.kind === finding.kind && known.box.test(finding.box);
}

/** Splits a screen's findings into new ones, and lists the known issues it no longer shows. */
export function compareWithKnown(screen: string, findings: OverflowFinding[]) {
  const known = KNOWN_OVERFLOW.filter((entry) => entry.screens.test(screen));
  return {
    unexpected: findings.filter((finding) => !known.some((entry) => matches(entry, finding))),
    gone: known.filter((entry) => !findings.some((finding) => matches(entry, finding))).map((entry) => entry.issue),
  };
}
