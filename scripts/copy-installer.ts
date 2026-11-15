// Copy the installer that `tauri build` just made into ./release/, so it is easy to find.
// Cargo may put its build output outside the repository (a shared `target-dir` in the user's Cargo config),
// so ask Cargo where that is instead of assuming src-tauri/target.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = join(root, "src-tauri", "Cargo.toml");

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
console.log(`Installer copied to ${relative(root, target).replaceAll("\\", "/")}`);
