// Draft the release notes for the version in src-tauri/tauri.conf.json from the commits since the last release,
// into release/notes-<version>.txt. Read and edit the draft, then pass it to `pnpm release:publish --notes`.
//
// The last release is the newest `v*` tag on main; `pnpm release:publish` makes those tags on GitHub, so they
// are fetched from origin first.
//
// Usage: pnpm release:notes [--since <ref>] [--force]
//   --since <ref>  list the commits after this tag or commit instead of the last release
//   --force        draft again over an existing file
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { type Commit, draftNotes, firstReleaseNotes, hasTodo } from "./release/draft-notes.ts";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { values: options } = parseArgs({
  options: {
    since: { type: "string" },
    force: { type: "boolean", default: false },
  },
});

function fail(message: string): never {
  console.error(`No draft written: ${message}`);
  process.exit(1);
}

// `quiet` keeps git's own error text out of the output, for calls whose failure is an expected answer.
function git(args: string[], quiet = false): string {
  return execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    ...(quiet ? { stdio: ["ignore", "pipe", "ignore"] as const } : {}),
  }).trim();
}

const shown = (path: string) => relative(root, path).replaceAll("\\", "/");

const config = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")) as { version?: string };
const version = config.version;
if (!version) fail("src-tauri/tauri.conf.json has no version.");

try {
  git(["fetch", "--quiet", "--tags", "origin"]);
} catch {
  fail("could not fetch the release tags from origin.");
}

let since = options.since;
if (!since) {
  try {
    since = git(["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", "HEAD"], true);
  } catch {
    // No release tag yet: this is the first release.
    since = undefined;
  }
}
if (since === `v${version}`) fail(`v${version} is already released. Raise the version first.`);

let notes: string;
let listed = 0;
if (since) {
  // Fields and records are split by the unit and record separators, which commit messages do not contain.
  const log = git(["log", "--reverse", "--no-merges", "--format=%s%x1f%b%x1e", `${since}..HEAD`]);
  const commits: Commit[] = log
    .split("\x1e")
    .map((record) => record.trim())
    .filter(Boolean)
    .map((record) => {
      const [subject = "", body = ""] = record.split("\x1f");
      return { subject, body };
    });
  notes = draftNotes(commits);
  listed = commits.length;
} else {
  notes = firstReleaseNotes(version);
}

const file = join(root, "release", `notes-${version}.txt`);
if (existsSync(file) && !options.force) {
  fail(`${shown(file)} already exists. Edit it, or draft again with --force.`);
}
mkdirSync(dirname(file), { recursive: true });
writeFileSync(file, notes);

console.log(
  since
    ? `Wrote ${shown(file)} from ${listed} commits since ${since}.`
    : `Wrote ${shown(file)}: there is no earlier release, so write the notes by hand.`,
);
if (hasTodo(notes)) console.log("Replace the TODO line before publishing; `pnpm release:publish` refuses it.");
console.log(`Read and edit it, then: pnpm release:publish --notes ${shown(file)}`);
