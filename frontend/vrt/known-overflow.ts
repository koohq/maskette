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

export const KNOWN_OVERFLOW: KnownOverflow[] = [
  {
    issue: "The project name field in the workspace title bar is too narrow for the name or its placeholder.",
    screens: /^workspace-(ja|en|zh-CN)-|^workspace-empty-(ja|en)-/,
    kind: "clipped",
    box: /ide-titlebar-title > input/,
  },
  {
    issue: "The language button is 40px tall in a shorter title bar.",
    screens: /^workspace/,
    kind: "spills",
    box: /header\.ide-titlebar$/,
  },
  {
    issue: "The canvas toolbar hides the zoom, fit and landmark controls that do not fit.",
    screens: /^workspace-(ja|en|zh-CN)-/,
    kind: "clipped",
    box: /editor-viewport-toolbar/,
  },
  {
    issue: '"Save as new version" runs past the editor toolbar into the assistant panel.',
    screens: /^workspace-en-min$/,
    kind: "spills",
    box: /div\.ide-toolbar$/,
  },
  {
    issue: "The hold-to-compare button sits below the bottom of the canvas area.",
    screens: /^workspace-(ja|en|zh-CN)-min$/,
    kind: "spills",
    box: /ide-editor-stage$/,
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
