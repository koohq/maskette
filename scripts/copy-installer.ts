// Copy the installer that `tauri build` just made into ./release/, so it is easy to find, and write the
// `latest.json` that installed copies read to find updates.
// Cargo may put its build output outside the repository (a shared `target-dir` in the user's Cargo config),
// so ask Cargo where that is instead of assuming src-tauri/target.
//
// Options:
//   --notes <file>   release notes for latest.json (shown in the app's update dialog), as plain text
import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = join(root, "src-tauri", "Cargo.toml");
const { values: options } = parseArgs({ options: { notes: { type: "string" } } });

const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps", "--manifest-path", manifest], {
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  }),
) as { target_directory: string };

const nsisDir = join(metadata.target_directory, "release", "bundle", "nsis");
let installers: string[];
try {
  installers = readdirSync(nsisDir).filter((name) => name.endsWith("-setup.exe"));
} catch {
  installers = [];
}
if (installers.length === 0) {
  console.error(`No installer (*-setup.exe) found in ${nsisDir}. Run \`pnpm bundle\` first.`);
  process.exit(1);
}

// Older versions can still sit next to the new one; take the one built last.
const newest = installers
  .map((name) => ({ name, time: statSync(join(nsisDir, name)).mtimeMs }))
  .sort((a, b) => b.time - a.time)[0];
if (!newest) process.exit(1);

const outDir = join(root, "release");
mkdirSync(outDir, { recursive: true });
const target = join(outDir, newest.name);
copyFileSync(join(nsisDir, newest.name), target);
// Keep the build's time: `pnpm release:publish` compares it with the last commit to the app.
utimesSync(target, new Date(), new Date(newest.time));
console.log(`Installer copied to ${shown(target)}`);

// The third-party notices that went into this build: scripts/third-party-notices.ts writes them into frontend/public/,
// and the screen's build carries them into frontend/dist/. The release attaches them.
const noticesTarget = join(outDir, "third-party-notices.txt");
rmSync(noticesTarget, { force: true });
const builtNotices = join(root, "frontend", "dist", "third-party-notices.txt");
if (existsSync(builtNotices)) {
  copyFileSync(builtNotices, noticesTarget);
  utimesSync(noticesTarget, new Date(), statSync(builtNotices).mtime);
  console.log(`Third-party notices copied to ${shown(noticesTarget)}`);
} else {
  console.warn(
    "This build has no third-party notices (frontend/dist/third-party-notices.txt); `pnpm bundle` writes them.",
  );
}

// The signature `tauri build` writes next to the installer when the signing key is set
// (TAURI_SIGNING_PRIVATE_KEY). One older than the installer belongs to an earlier build of the same version.
const signature = join(nsisDir, `${newest.name}.sig`);
if (!existsSync(signature) || statSync(signature).mtimeMs < newest.time) {
  // Files from an earlier signed build must not be published with this one.
  rmSync(join(outDir, "latest.json"), { force: true });
  rmSync(`${target}.sig`, { force: true });
  console.warn("No signature for this installer, so latest.json was not written (the build had no signing key).");
  process.exit(0);
}

const version = /_(\d+\.\d+\.\d+[^_]*)_[^_]+-setup\.exe$/.exec(newest.name)?.[1];
if (!version) {
  console.error(`Could not read the version from ${newest.name}.`);
  process.exit(1);
}

// Installed copies ask the endpoint in tauri.conf.json, `.../releases/latest/download/latest.json`; the installer
// is attached to the same GitHub release, whose tag is `v<version>`.
const config = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")) as {
  plugins?: { updater?: { endpoints?: string[] } };
};
const endpoint = config.plugins?.updater?.endpoints?.[0] ?? "";
const releases = /^(https:\/\/github\.com\/[^/]+\/[^/]+\/releases)\/latest\/download\/latest\.json$/.exec(
  endpoint,
)?.[1];
if (!releases) {
  console.error(`The updater endpoint in tauri.conf.json is not a GitHub "latest release" URL: ${endpoint}`);
  process.exit(1);
}

copyFileSync(signature, `${target}.sig`);
const latest = {
  version,
  ...(options.notes ? { notes: readFileSync(options.notes, "utf8").trim() } : {}),
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  platforms: {
    "windows-x86_64": {
      signature: readFileSync(signature, "utf8").trim(),
      url: `${releases}/download/v${version}/${newest.name}`,
    },
  },
};
const latestPath = join(outDir, "latest.json");
writeFileSync(latestPath, `${JSON.stringify(latest, null, 2)}\n`);
console.log(
  `Wrote ${shown(latestPath)} for version ${version}${options.notes ? "" : " (no release notes: --notes <file>)"}`,
);

function shown(path: string): string {
  return relative(root, path).replaceAll("\\", "/");
}
