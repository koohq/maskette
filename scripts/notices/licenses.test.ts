import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it } from "node:test";
import { chooseLicenses, classifyText, normalizeText, parseExpression, personName, withCopyright } from "./licenses.ts";

const stock = (id: string) => readFileSync(join(import.meta.dirname, "..", "..", "LICENSES", `${id}.txt`), "utf8");

describe("parseExpression", () => {
  it("reads OR, AND, parentheses and WITH", () => {
    assert.deepEqual(parseExpression("(MIT OR Apache-2.0) AND Unicode-3.0"), {
      kind: "and",
      parts: [
        {
          kind: "or",
          parts: [
            { kind: "id", id: "MIT" },
            { kind: "id", id: "Apache-2.0" },
          ],
        },
        { kind: "id", id: "Unicode-3.0" },
      ],
    });
    assert.deepEqual(parseExpression("Apache-2.0 WITH LLVM-exception OR MIT"), {
      kind: "or",
      parts: [
        { kind: "id", id: "Apache-2.0 WITH LLVM-exception" },
        { kind: "id", id: "MIT" },
      ],
    });
  });

  it("takes the old slash for OR", () => {
    assert.deepEqual(parseExpression("MIT/Apache-2.0"), parseExpression("MIT OR Apache-2.0"));
  });

  it("refuses what it cannot read", () => {
    assert.throws(() => parseExpression("(MIT OR Apache-2.0"));
    assert.throws(() => parseExpression("MIT Apache-2.0"));
  });
});

describe("chooseLicenses", () => {
  const all = () => true;

  it("takes the preferred one of an OR", () => {
    assert.deepEqual(chooseLicenses(parseExpression("Apache-2.0 OR MIT"), all), ["MIT"]);
    assert.deepEqual(chooseLicenses(parseExpression("Unlicense OR MIT"), all), ["MIT"]);
    assert.deepEqual(chooseLicenses(parseExpression("MPL-2.0 OR BSD-3-Clause"), all), ["BSD-3-Clause"]);
  });

  it("takes every part of an AND", () => {
    assert.deepEqual(chooseLicenses(parseExpression("(MIT OR Apache-2.0) AND Unicode-3.0"), all), [
      "MIT",
      "Unicode-3.0",
    ]);
  });

  it("prefers a license whose text the package carries", () => {
    const onlyApache = (id: string) => id === "Apache-2.0";
    assert.deepEqual(chooseLicenses(parseExpression("MIT OR Apache-2.0"), onlyApache), ["Apache-2.0"]);
  });

  it("passes over a license that is not accepted", () => {
    assert.deepEqual(chooseLicenses(parseExpression("GPL-2.0-only OR MIT"), all), ["MIT"]);
    assert.deepEqual(chooseLicenses(parseExpression("GPL-2.0-only"), all), ["GPL-2.0-only"]);
  });
});

describe("classifyText", () => {
  it("knows the stock texts", () => {
    for (const id of ["MIT", "Apache-2.0", "BSD-3-Clause", "ISC", "MPL-2.0"]) {
      assert.deepEqual(classifyText(stock(id)), [id], id);
    }
  });

  it("reads a text however its lines are wrapped", () => {
    const rewrapped = stock("MIT")
      .replace(/\n(?!\n)/g, " ")
      .replace(/ {2,}/g, "\n   ");
    assert.deepEqual(classifyText(rewrapped), ["MIT"]);
  });

  it("tells the two-clause BSD and 0BSD from the longer ones", () => {
    const twoClause = stock("BSD-3-Clause").replace(/3\. Neither[\s\S]*?permission\.\n/, "");
    assert.deepEqual(classifyText(twoClause), ["BSD-2-Clause"]);
    const zeroBsd =
      "Permission to use, copy, modify, and/or distribute this software for any\npurpose with or without fee is hereby granted.\n\nTHE SOFTWARE IS PROVIDED";
    assert.deepEqual(classifyText(zeroBsd), ["0BSD"]);
  });

  it("finds both halves of a dual license in one file", () => {
    assert.deepEqual(classifyText(`${stock("MIT")}\n\n${stock("Apache-2.0")}`), ["Apache-2.0", "MIT"]);
  });

  it("does not take a pointer to a license for the license", () => {
    assert.deepEqual(classifyText('Licensed under the Apache License, Version 2.0 (the "License").'), []);
    assert.deepEqual(
      classifyText("This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0."),
      [],
    );
  });
});

describe("texts", () => {
  it("normalizes line ends, trailing spaces and blank edges", () => {
    assert.equal(normalizeText("﻿\r\n\r\nMIT License  \r\n\r\nCopyright\r\n\r\n"), "MIT License\n\nCopyright");
  });

  it("removes the indent every line shares, and only that", () => {
    assert.equal(
      normalizeText("    MIT License\n\n      Copyright\n    Permission"),
      "MIT License\n\n  Copyright\nPermission",
    );
  });

  it("puts the holder into a stock text", () => {
    const mit = withCopyright(stock("MIT"), "2021 Jane Doe");
    assert.match(mit, /^MIT License\n\nCopyright \(c\) 2021 Jane Doe\n\nPermission/);
    const apache = withCopyright(stock("Apache-2.0"), "Example Inc.");
    assert.match(apache, /^Copyright \(c\) Example Inc\.\n\n {0,40}Apache License/);
  });

  it("keeps only a person's name", () => {
    assert.equal(personName("Jane Doe <jane@example.com> (https://example.com)"), "Jane Doe");
  });
});
