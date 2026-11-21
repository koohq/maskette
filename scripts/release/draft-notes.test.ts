import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { type Commit, draftNotes, firstReleaseNotes, hasTodo } from "./draft-notes.ts";

const commit = (subject: string, body = ""): Commit => ({ subject, body });

describe("draftNotes", () => {
  it("lists feat commits under New and fix commits under Fixes, oldest first", () => {
    const notes = draftNotes([
      commit("feat: add the eye close-up"),
      commit("fix: keep the crop in place"),
      commit("feat(editor): undo a moved landmark"),
      commit("fix(update)!: ask before restarting"),
    ]);
    assert.equal(
      notes,
      [
        "New",
        "- Add the eye close-up",
        "- Undo a moved landmark",
        "",
        "Fixes",
        "- Keep the crop in place",
        "- Breaking: Ask before restarting",
        "",
      ].join("\n"),
    );
  });

  it("leaves out internal commit types and commits without a type", () => {
    const notes = draftNotes([
      commit("ci: bump the actions group with 1 update"),
      commit("chore: try the notes against the real model"),
      commit("docs: reword the README"),
      commit("refactor: split the router"),
      commit("test: cover the empty project"),
      commit("Merge branch 'main'"),
      commit("fix: say when the account is out of credits"),
    ]);
    assert.equal(notes, "Fixes\n- Say when the account is out of credits\n");
  });

  it("marks a commit whose body declares a breaking change", () => {
    const notes = draftNotes([commit("feat: store projects in one file", "BREAKING CHANGE: old exports do not open")]);
    assert.equal(notes, "New\n- Breaking: Store projects in one file\n");
  });

  it("drops a commit that a later commit reverted, together with the revert", () => {
    const notes = draftNotes([
      commit("feat: add the size fields"),
      commit("fix: keep the crop in place"),
      commit('Revert "feat: add the size fields"', "This reverts commit 1234567."),
    ]);
    assert.equal(notes, "Fixes\n- Keep the crop in place\n");
  });

  it("asks for a sentence when nothing user-facing changed", () => {
    const notes = draftNotes([commit("ci: bump the actions group with 2 updates")]);
    assert.ok(hasTodo(notes));
  });
});

describe("firstReleaseNotes", () => {
  it("is a reminder to write the notes by hand", () => {
    const notes = firstReleaseNotes("0.1.0");
    assert.match(notes, /0\.1\.0/);
    assert.ok(hasTodo(notes));
  });
});

describe("hasTodo", () => {
  it("finds the marker only at the start of a line", () => {
    assert.ok(hasTodo("New\n  TODO: say more\n"));
    assert.ok(!hasTodo("New\n- Explain what TODO: means in the editor\n"));
    assert.ok(!hasTodo("Fixes\n- Keep the crop in place\n"));
  });
});
