// Reading license expressions and license files, for the third-party notices (scripts/third-party-notices.ts).

/**
 * The licenses a bundled component may come under, most preferred first: of an "A OR B", the one earlier here is
 * taken. Anything else stops the notices until someone has looked at it.
 */
export const ACCEPTED = [
  "MIT",
  "Apache-2.0",
  "BSD-3-Clause",
  "BSD-2-Clause",
  "ISC",
  "Zlib",
  "0BSD",
  "MIT-0",
  "Unlicense",
  "BSL-1.0",
  "CC0-1.0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "MPL-2.0",
] as const;

export type Expression = { kind: "id"; id: string } | { kind: "and" | "or"; parts: Expression[] };

/** Parses an SPDX expression. Old Cargo manifests write "MIT/Apache-2.0" for "MIT OR Apache-2.0". */
export function parseExpression(text: string): Expression {
  const tokens = text.replaceAll("/", " OR ").replaceAll("(", " ( ").replaceAll(")", " ) ").trim().split(/\s+/);
  let position = 0;
  const peek = () => tokens[position]?.toUpperCase();
  const take = () => {
    const token = tokens[position++];
    if (token === undefined || token === "") throw new Error(`The license expression ends too early: ${text}`);
    return token;
  };
  const either = (): Expression => {
    const parts = [both()];
    while (peek() === "OR") {
      position++;
      parts.push(both());
    }
    return parts.length === 1 && parts[0] ? parts[0] : { kind: "or", parts };
  };
  const both = (): Expression => {
    const parts = [single()];
    while (peek() === "AND") {
      position++;
      parts.push(single());
    }
    return parts.length === 1 && parts[0] ? parts[0] : { kind: "and", parts };
  };
  const single = (): Expression => {
    const token = take();
    if (token === "(") {
      const inner = either();
      if (take() !== ")") throw new Error(`A parenthesis is not closed in the license expression: ${text}`);
      return inner;
    }
    if (peek() === "WITH") {
      position++;
      return { kind: "id", id: `${token} WITH ${take()}` };
    }
    return { kind: "id", id: token };
  };
  const expression = either();
  if (position !== tokens.length) throw new Error(`Could not read the license expression: ${text}`);
  return expression;
}

/** "Apache-2.0 WITH LLVM-exception" → "Apache-2.0": the license whose text applies. */
export function baseId(id: string): string {
  return id.split(" WITH ")[0] ?? id;
}

export function isAccepted(id: string): boolean {
  return (ACCEPTED as readonly string[]).includes(baseId(id));
}

/** Where a license stands in {@link ACCEPTED}; one not there comes last. */
export function rank(id: string): number {
  const index = (ACCEPTED as readonly string[]).indexOf(baseId(id));
  return index === -1 ? ACCEPTED.length : index;
}

/**
 * The licenses to follow for an expression: all of an "AND", one of an "OR". Of an "OR" the choice is the accepted
 * one whose text the package itself carries (`hasText`), then the one earliest in {@link ACCEPTED}.
 */
export function chooseLicenses(expression: Expression, hasText: (id: string) => boolean): string[] {
  if (expression.kind === "id") return [expression.id];
  const choices = expression.parts.map((part) => chooseLicenses(part, hasText));
  if (expression.kind === "and") return [...new Set(choices.flat())];
  const score = (ids: string[]) => [
    ids.every(isAccepted) ? 0 : 1,
    ids.every((id) => hasText(baseId(id))) ? 0 : 1,
    Math.max(...ids.map(rank)),
  ];
  const ranked = choices
    .map((ids) => ({ ids, score: score(ids) }))
    .sort((a, b) => {
      for (const [index, value] of a.score.entries()) {
        const other = b.score[index] ?? 0;
        if (value !== other) return value - other;
      }
      return 0;
    });
  return ranked[0]?.ids ?? [];
}

/**
 * Each license's wording, matched on the text with its line breaks and runs of spaces made single spaces. Apache-2.0
 * and MPL-2.0 need their terms, not just their name: a file that only says "Licensed under the Apache License" is no
 * copy of the license.
 */
const SIGNATURES: [id: string, wording: RegExp][] = [
  ["Apache-2.0", /Apache License,? Version 2\.0.*TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION/i],
  [
    "MIT",
    /Permission is hereby granted, free of charge, to any person obtaining a copy.*the above copyright notice and this permission notice shall be included/i,
  ],
  ["MPL-2.0", /Mozilla Public License Version 2\.0.*Definitions/i],
  [
    "BSD-3-Clause",
    /Redistribution and use in source and binary forms.*(?:Neither the name|names of (?:its|the) contributors)/i,
  ],
  ["BSD-2-Clause", /Redistribution and use in source and binary forms/i],
  [
    "ISC",
    /Permission to use, copy, modify, and(?:\/or)? distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice/i,
  ],
  [
    "0BSD",
    /Permission to use, copy, modify, and\/or distribute this software for any purpose with or without fee is hereby granted\./i,
  ],
  [
    "Zlib",
    /This software is provided ['‘]as-is['’].*Permission is granted to anyone to use this software for any purpose/i,
  ],
  ["Unicode-3.0", /UNICODE LICENSE V3/i],
  ["Unicode-DFS-2016", /UNICODE, INC\. LICENSE AGREEMENT/i],
  ["Unlicense", /This is free and unencumbered software released into the public domain/i],
  ["BSL-1.0", /Boost Software License/i],
  ["CC0-1.0", /CC0 1\.0 Universal/i],
  ["Bitstream-Vera", /Bitstream Vera/i],
];

/** The licenses a license file's text reads as. A file may hold several (both halves of a dual license). */
export function classifyText(text: string): string[] {
  const flat = text.replace(/\s+/g, " ");
  const found = SIGNATURES.filter(([, wording]) => wording.test(flat)).map(([id]) => id);
  // The wording of the shorter license also appears in the longer one.
  const shadowed: Record<string, string> = { "BSD-2-Clause": "BSD-3-Clause", "0BSD": "ISC" };
  return found.filter((id) => {
    const longer = shadowed[id];
    return longer === undefined || !found.includes(longer);
  });
}

/**
 * One text however it was saved: no byte-order mark, LF line ends, no spaces at line ends, no blank edges, and no
 * indent that every line shares.
 */
export function normalizeText(text: string): string {
  const lines = text
    .replace(/^﻿/, "")
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.trimEnd());
  const indents = lines.filter((line) => line !== "").map((line) => /^[ \t]*/.exec(line)?.[0].length ?? 0);
  const shared = indents.length > 0 ? Math.min(...indents) : 0;
  return lines
    .map((line) => line.slice(shared))
    .join("\n")
    .replace(/^\n+|\n+$/g, "");
}

const PLACEHOLDER = /^Copyright \(c\) <year> <copyright holders>$/m;

/**
 * A stock license text (`LICENSES/<id>.txt`) for a component whose package carries none, with the holder put in.
 * Texts without a copyright line of their own (Apache-2.0, MPL-2.0) get one on top.
 */
export function withCopyright(template: string, holder: string): string {
  const line = `Copyright (c) ${holder}`;
  const text = normalizeText(template);
  return PLACEHOLDER.test(text) ? text.replace(PLACEHOLDER, line) : `${line}\n\n${text}`;
}

/** "Jane Doe <jane@example.com> (https://example.com)" → "Jane Doe". */
export function personName(person: string): string {
  return person
    .replace(/<[^>]*>/g, "")
    .replace(/\([^)]*\)/g, "")
    .trim();
}
