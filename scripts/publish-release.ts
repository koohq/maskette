// Publish the installer `pnpm bundle` built as a GitHub release, with the latest.json installed copies read.
//
// Run it after `pnpm bundle` (with the signing key set), on a clean main that is pushed. It writes latest.json
// with the release notes, checks that the pieces belong together, shows what it will publish and asks first.
//
// Usage: pnpm release:publish --notes <file> [--dry-run] [--yes]
//   --notes <file>  release notes as plain text, shown in the app's update dialog and on the release page
//                   (`pnpm release:notes` drafts them; a line still starting with "TODO:" stops the release)
//   --dry-run       check and show everything, but publish nothing
//   --yes           publish without asking
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { stdin, stdout } from "node:process";
import { createInterface } from "node:readline/promises";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { signatureProblem } from "./minisign.ts";
import { hasTodo, TODO_MARKER } from "./release/draft-notes.ts";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { values: options } = parseArgs({
  options: {
    notes: { type: "string" },
    "dry-run": { type: "boolean", default: false },
    yes: { type: "boolean", default: false },
  },
});

function fail(message: string): never {
  console.error(`Not published: ${message}`);
  process.exit(1);
}

function run(command: string, args: string[]): string {
  return execFileSync(command, args, { cwd: root, encoding: "utf8" }).trim();
}

if (!options.notes) fail("give the release notes with --notes <file>.");
const notesFile = resolve(options.notes);
if (!existsSync(notesFile)) fail(`there is no notes file at ${options.notes}.`);
const notes = readFileSync(notesFile, "utf8").trim();
if (!notes) fail("the notes file is empty.");
if (hasTodo(notes)) fail(`the notes still have a line starting with "${TODO_MARKER}". Write it, then publish.`);

// tauri.conf.json's version is the one the updater compares; Cargo's must say the same.
const config = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")) as {
  version?: string;
  plugins?: { updater?: { pubkey?: string; endpoints?: string[] } };
};
const version = config.version ?? "";
const cargoVersion = /^version = "([^"]+)"/m.exec(readFileSync(join(root, "src-tauri", "Cargo.toml"), "utf8"))?.[1];
if (!version || version !== cargoVersion) {
  fail(`tauri.conf.json says ${version || "no version"} and src-tauri/Cargo.toml says ${cargoVersion ?? "none"}.`);
}
const tag = `v${version}`;
const endpoint = config.plugins?.updater?.endpoints?.[0] ?? "";
const repo = /^https:\/\/github\.com\/([^/]+\/[^/]+)\/releases\/latest\/download\/latest\.json$/.exec(endpoint)?.[1];
if (!repo) fail(`the updater endpoint in tauri.conf.json is not a GitHub release URL: ${endpoint}`);
const pubkey = config.plugins?.updater?.pubkey;
if (!pubkey) fail("tauri.conf.json has no updater public key.");

// Made from main as it is on GitHub, with nothing left uncommitted.
if (run("git", ["status", "--porcelain"])) {
  fail("there are uncommitted changes. Commit and push them, then build again.");
}
if (run("git", ["rev-parse", "--abbrev-ref", "HEAD"]) !== "main") fail("releases are made from main.");
run("git", ["fetch", "--quiet", "origin", "main"]);
const head = run("git", ["rev-parse", "HEAD"]);
if (head !== run("git", ["rev-parse", "origin/main"])) fail("main differs from origin/main. Push (or pull) first.");
if (run("git", ["ls-remote", "--tags", "origin", `refs/tags/${tag}`])) {
  fail(`${tag} already exists on GitHub. Raise the version and build again.`);
}

// latest.json for the newest build, with the notes.
execFileSync(process.execPath, [join(root, "scripts", "copy-installer.ts"), "--notes", notesFile], {
  cwd: root,
  stdio: "inherit",
});
const latestPath = join(root, "release", "latest.json");
if (!existsSync(latestPath)) {
  fail("the build has no signature. Set TAURI_SIGNING_PRIVATE_KEY and its password, then run `pnpm bundle`.");
}
const latest = JSON.parse(readFileSync(latestPath, "utf8")) as {
  version: string;
  platforms: Record<string, { url: string } | undefined>;
};
if (latest.version !== version) fail(`the newest build is ${latest.version}, not ${version}. Run \`pnpm bundle\`.`);
const installerName = latest.platforms["windows-x86_64"]?.url.split("/").pop() ?? "";
const installer = join(root, "release", installerName);
const signatureFile = `${installer}.sig`;

// Built after the last commit to the app, so it is what main holds.
const appPaths = ["src-tauri", "frontend", "package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml"];
const lastChange = Number(run("git", ["log", "-1", "--format=%ct", "--", ...appPaths])) * 1000;
if (statSync(installer).mtimeMs < lastChange) {
  fail("the installer is older than the last commit to the app. Run `pnpm bundle` again.");
}

// The third-party notices that went into the same build, for anyone who reads them before installing.
const noticesFile = join(root, "release", "third-party-notices.txt");
if (!existsSync(noticesFile)) fail("the build has no third-party notices. Run `pnpm bundle`, which writes them.");
const noticesVersion = /^Third-party notices for Maskette (\S+)$/m.exec(readFileSync(noticesFile, "utf8"))?.[1];
if (noticesVersion !== version || statSync(noticesFile).mtimeMs < lastChange) {
  fail("the third-party notices are not from this build. Run `pnpm bundle` again.");
}

// Signed with the key installed copies trust; anything else they would refuse to install.
const problem = signatureProblem(readFileSync(installer), readFileSync(signatureFile, "utf8"), pubkey);
if (problem) fail(`the installer's signature does not check out with the updater's public key: ${problem}.`);

const sizeMiB = (statSync(installer).size / 1024 / 1024).toFixed(2);
console.log(`
Release ${tag} on ${repo}
  commit: ${run("git", ["log", "-1", "--format=%h %s"])}
  files:  ${installerName} (${sizeMiB} MiB), ${installerName}.sig, latest.json, third-party-notices.txt
  notes:
${notes
  .split(/\r?\n/)
  .map((line) => `    ${line}`)
  .join("\n")}
`);

if (options["dry-run"]) {
  console.log("Dry run: nothing was published.");
  process.exit(0);
}
if (!options.yes) {
  const prompt = createInterface({ input: stdin, output: stdout });
  const answer = await prompt.question("Publish it? Installed copies will offer this update. (y/N) ");
  prompt.close();
  if (answer.trim().toLowerCase() !== "y") {
    console.log("Not published.");
    process.exit(0);
  }
}

const created = spawnSync(
  "gh",
  [
    "release",
    "create",
    tag,
    installer,
    signatureFile,
    latestPath,
    noticesFile,
    "--repo",
    repo,
    "--target",
    head,
    "--title",
    `Maskette ${version}`,
    "--notes-file",
    notesFile,
  ],
  { cwd: root, stdio: "inherit" },
);
if (created.status !== 0) fail("`gh release create` failed (see above).");
console.log(`Published ${tag}. Installed copies offer it at their next launch.`);
