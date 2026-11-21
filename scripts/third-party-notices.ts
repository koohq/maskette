// Write the third-party notices: every component this build of Maskette ships that others made, with the license it
// comes under and that license's text.
//
// `pnpm bundle` runs this before `tauri build`, and `pnpm notices` runs it alone. It writes two files into
// frontend/public/, so they go into the app with the screen:
//   third-party-notices.json  read by the app's "Third-party licenses" page
//   third-party-notices.txt   the same as plain text; `pnpm release:copy` puts it next to the installer and
//                             `pnpm release:publish` attaches it to the release
// Neither is committed: both follow the lockfiles and the machine's Rust toolchain.
//
// The components:
//   - the Rust crates compiled into maskette.exe (normal dependencies for this machine's target; proc-macros only run
//     while compiling, so they are left out), and the Rust standard library
//   - the npm packages the screen uses (the dependencies of frontend/package.json, followed through node_modules)
//   - the model files, the DejaVu fonts, and the parts of the NSIS installer, written out below
// It stops, listing them, when a component's license is not one of the accepted ones (scripts/notices/licenses.ts) or
// no text for it is found.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  baseId,
  chooseLicenses,
  classifyText,
  isAccepted,
  normalizeText,
  parseExpression,
  personName,
  rank,
  withCopyright,
} from "./notices/licenses.ts";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "frontend", "public");

type Group = "app" | "screen" | "data" | "installer";
type Component = {
  name: string;
  version?: string;
  group: Group;
  license: string;
  url: string;
  note?: string;
  /** Indexes into the texts. */
  texts: number[];
};

const GROUP_TITLES: Record<Group, string> = {
  app: "The app (Rust)",
  screen: "The screen (JavaScript)",
  data: "Models and fonts",
  installer: "The installer",
};

const texts: string[] = [];
const textIndexes = new Map<string, number>();
const components: Component[] = [];
const problems: string[] = [];

/** A license text's number; one text that several components share is kept once. */
function textNumber(text: string): number {
  const normalized = normalizeText(text);
  let index = textIndexes.get(normalized);
  if (index === undefined) {
    index = texts.length;
    texts.push(normalized);
    textIndexes.set(normalized, index);
  }
  return index;
}

/** A stock text from LICENSES/, for a component whose package carries none. */
function stock(id: string, holder: string): string | undefined {
  const path = join(root, "LICENSES", `${baseId(id)}.txt`);
  return existsSync(path) ? withCopyright(readFileSync(path, "utf8"), holder) : undefined;
}

function addComponent(component: Omit<Component, "texts">, componentTexts: string[]) {
  components.push({ ...component, texts: [...new Set(componentTexts.map(textNumber))] });
}

/** Files that hold a license, a notice or a copyright statement, at the top of a package. */
const LICENSE_FILE = /^(?:licen[cs]e|copying|copyright|notice|unlicense)(?:[-_.][\w.-]*)?$/i;
/** Code that happens to have such a name, like the "Copyright" icon of @mui/icons-material. */
const NOT_TEXT = /\.(?:[cm]?[jt]sx?|json|html?|rs|toml|map|css|svg|png)$/i;
/** Apache-2.0 asks for the NOTICE file to go along, and a COPYRIGHT file says whose the package is. */
const ALWAYS_KEPT = /^(?:notice|copyright)/i;

type Package = {
  name: string;
  version: string;
  group: Group;
  /** The SPDX expression; null when the package only names a license file (Cargo's `license-file`). */
  declared: string | null;
  dir: string;
  extraFiles?: string[];
  holder: string;
  url: string;
  note?: string | undefined;
};

function addPackage(pkg: Package) {
  const where = `${pkg.name} ${pkg.version}`;
  const names = readdirSync(pkg.dir).filter(
    (name) => LICENSE_FILE.test(name) && !NOT_TEXT.test(name) && statSync(join(pkg.dir, name)).isFile(),
  );
  const paths = [...new Set([...names.map((name) => join(pkg.dir, name)), ...(pkg.extraFiles ?? [])])];
  const files = paths.map((path) => {
    const text = normalizeText(readFileSync(path, "utf8"));
    return { name: relative(pkg.dir, path), text, ids: classifyText(text) };
  });
  const carried = new Set(files.flatMap((file) => file.ids));

  if (pkg.declared === null) {
    const ids = [...carried];
    if (ids.length === 0 || !ids.every(isAccepted)) {
      problems.push(`${where}: its license file reads as ${ids.join(" AND ") || "nothing known"}`);
    }
    addComponent(
      { name: pkg.name, version: pkg.version, group: pkg.group, license: ids.join(" AND "), url: pkg.url },
      files.map((file) => file.text),
    );
    return;
  }

  let chosen: string[];
  try {
    chosen = chooseLicenses(parseExpression(pkg.declared), (id) => carried.has(id));
  } catch (error) {
    problems.push(`${where}: ${(error as Error).message}`);
    return;
  }
  const refused = chosen.filter((id) => !isAccepted(id));
  if (refused.length > 0) problems.push(`${where}: ${pkg.declared} (not accepted: ${refused.join(", ")})`);

  const wanted = new Set(chosen.map(baseId));
  const picked = files.filter((file) => ALWAYS_KEPT.test(file.name) || file.ids.some((id) => wanted.has(id)));
  const covered = new Set(picked.flatMap((file) => file.ids));
  const uncovered = chosen.filter((id) => !covered.has(baseId(id)));
  const componentTexts = picked.map((file) => file.text);
  if (uncovered.length > 0) {
    // A license file in words not recognized may still be the license, or at least say whose the package is.
    for (const file of files) {
      if (file.ids.length === 0 && !ALWAYS_KEPT.test(file.name)) componentTexts.push(file.text);
    }
    for (const id of uncovered) {
      const text = stock(id, pkg.holder);
      if (text === undefined) problems.push(`${where}: no text for ${id} (add LICENSES/${baseId(id)}.txt)`);
      else componentTexts.push(text);
    }
  }
  let note = pkg.note;
  if (wanted.has("MPL-2.0")) {
    note = [note, "Its source code is at the link above."].filter(Boolean).join(" ");
  }
  addComponent(
    {
      name: pkg.name,
      version: pkg.version,
      group: pkg.group,
      license: [...chosen].sort((a, b) => rank(a) - rank(b)).join(" AND "),
      url: pkg.url,
      ...(note ? { note } : {}),
    },
    componentTexts,
  );
}

// ---- Rust

type CargoPackage = {
  id: string;
  name: string;
  version: string;
  license: string | null;
  license_file: string | null;
  manifest_path: string;
  authors: string[];
  targets: { kind: string[] }[];
};
type CargoMetadata = {
  packages: CargoPackage[];
  resolve: { root: string; nodes: { id: string; deps: { pkg: string; dep_kinds: { kind: string | null }[] }[] }[] };
};

const rustc = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
const host = /^host: (\S+)$/m.exec(rustc)?.[1];
const rustVersion = /^release: (\S+)$/m.exec(rustc)?.[1];
if (!host || !rustVersion) throw new Error(`Could not read the host and version from \`rustc -vV\`:\n${rustc}`);

const metadata = JSON.parse(
  execFileSync(
    "cargo",
    [
      "metadata",
      "--format-version",
      "1",
      "--locked",
      "--filter-platform",
      host,
      "--manifest-path",
      join(root, "src-tauri", "Cargo.toml"),
    ],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  ),
) as CargoMetadata;

const crates = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
const linked = new Set<string>();
const pending = [metadata.resolve.root];
while (pending.length > 0) {
  const id = pending.pop();
  if (id === undefined || linked.has(id)) continue;
  linked.add(id);
  for (const dep of nodes.get(id)?.deps ?? []) {
    if (dep.dep_kinds.some((kind) => kind.kind === null)) pending.push(dep.pkg);
  }
}
linked.delete(metadata.resolve.root);

/** Crate notes by name. */
const CRATE_NOTES: Record<string, string> = {
  dejavu: "Embeds the DejaVu fonts, listed under “Models and fonts”.",
};

let dejavuDir: string | undefined;
for (const id of linked) {
  const crate = crates.get(id);
  if (!crate || crate.targets.some((target) => target.kind.includes("proc-macro"))) continue;
  const dir = dirname(crate.manifest_path);
  if (crate.name === "dejavu") dejavuDir = dir;
  const authors = crate.authors.map(personName).filter(Boolean);
  addPackage({
    name: crate.name,
    version: crate.version,
    group: "app",
    declared: crate.license,
    dir,
    ...(crate.license_file ? { extraFiles: [resolve(dir, crate.license_file)] } : {}),
    holder: authors.length > 0 ? authors.join(", ") : `The ${crate.name} developers`,
    url: `https://crates.io/crates/${crate.name}/${crate.version}`,
    note: CRATE_NOTES[crate.name],
  });
}

const mitStock = (holder: string) => {
  const text = stock("MIT", holder);
  if (text === undefined) throw new Error("LICENSES/MIT.txt is missing.");
  return text;
};
const apacheStock = (holder: string) => {
  const text = stock("Apache-2.0", holder);
  if (text === undefined) throw new Error("LICENSES/Apache-2.0.txt is missing.");
  return text;
};

addComponent(
  {
    name: "Rust standard library",
    version: rustVersion,
    group: "app",
    license: "MIT",
    url: "https://github.com/rust-lang/rust",
    note: "Offered under MIT OR Apache-2.0; compiled into the app.",
  },
  [mitStock("The Rust Project Contributors")],
);

// ---- npm

type NpmPerson = string | { name?: string };
type NpmManifest = {
  name: string;
  version: string;
  license?: string | { type?: string };
  licenses?: { type?: string }[];
  author?: NpmPerson;
  contributors?: NpmPerson[];
  dependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
};

/** Copyright holders where the package's own author field names a maintainer, not the owner. */
const NPM_HOLDERS: Record<string, string> = {
  "onnxruntime-common": "Microsoft Corporation",
  "onnxruntime-web": "Microsoft Corporation",
  "@mediapipe/tasks-vision": "The MediaPipe Authors",
};
const NPM_NOTES: Record<string, (version: string) => string> = {
  "onnxruntime-web": () =>
    "Its WebAssembly module also holds third-party components, which ONNX Runtime lists in its ThirdPartyNotices.txt.",
  "@mediapipe/tasks-vision": (version) =>
    "Its WebAssembly module is built with third-party components; see the third_party folder of the MediaPipe " +
    `source: https://github.com/google-ai-edge/mediapipe/tree/v${version}/third_party`,
};

/** Where Node would find `name` from `from`: the nearest node_modules on the way up. */
function resolvePackage(name: string, from: string): string | undefined {
  let dir = from;
  for (;;) {
    const candidate = join(dir, "node_modules", name, "package.json");
    if (existsSync(candidate)) return realpathSync(dirname(candidate));
    const parent = dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

function personOf(person: NpmPerson | undefined): string {
  if (person === undefined) return "";
  return personName(typeof person === "string" ? person : (person.name ?? ""));
}

function npmLicense(manifest: NpmManifest): string | null {
  if (typeof manifest.license === "string") return manifest.license;
  if (manifest.license?.type) return manifest.license.type;
  const old = (manifest.licenses ?? []).map((license) => license.type).filter(Boolean);
  return old.length > 0 ? old.join(" OR ") : null;
}

const frontend = join(root, "frontend");
const frontendManifest = JSON.parse(readFileSync(join(frontend, "package.json"), "utf8")) as NpmManifest;
const visited = new Set<string>();
const queue: { name: string; from: string; optional: boolean }[] = [
  ...Object.keys(frontendManifest.dependencies ?? {}).map((name) => ({ name, from: frontend, optional: false })),
];
while (queue.length > 0) {
  const next = queue.shift();
  if (!next) break;
  const dir = resolvePackage(next.name, next.from);
  if (dir === undefined) {
    if (!next.optional) problems.push(`${next.name}: not found in node_modules (run \`pnpm install\`)`);
    continue;
  }
  if (visited.has(dir)) continue;
  visited.add(dir);
  const manifest = JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as NpmManifest;
  const declared = npmLicense(manifest);
  const holder =
    NPM_HOLDERS[manifest.name] ||
    personOf(manifest.author) ||
    personOf(manifest.contributors?.[0]) ||
    `The ${manifest.name} authors`;
  if (declared === null) problems.push(`${manifest.name} ${manifest.version}: no license in package.json`);
  else {
    addPackage({
      name: manifest.name,
      version: manifest.version,
      group: "screen",
      declared,
      dir,
      holder,
      url: `https://www.npmjs.com/package/${manifest.name}/v/${manifest.version}`,
      note: NPM_NOTES[manifest.name]?.(manifest.version),
    });
  }
  for (const name of Object.keys(manifest.dependencies ?? {})) queue.push({ name, from: dir, optional: false });
  for (const name of Object.keys(manifest.optionalDependencies ?? {})) queue.push({ name, from: dir, optional: true });
}

// ---- Models and fonts (files kept in this repository, and the fonts the dejavu crate embeds)

addComponent(
  {
    name: "Anime face landmark model (anime-face-detector, HRNetV2)",
    version: "0.0.1",
    group: "data",
    license: "MIT",
    url: "https://github.com/hysts/anime-face-detector",
    note:
      "anime-face-hrnetv2-int8.onnx: the weights of mmpose_anime-face_hrnetv2.pth from the v0.0.1 release, " +
      "converted to ONNX and quantized to 8-bit integers (as found in Visual Studio Kigurumi).",
  },
  [mitStock("2021 hysts")],
);
addComponent(
  {
    name: "BlazeFace short-range face detection model (MediaPipe)",
    group: "data",
    license: "Apache-2.0",
    url: "https://ai.google.dev/edge/mediapipe/solutions/vision/face_detector",
    note: "blaze_face_short_range.tflite (float16, version 1).",
  },
  [apacheStock("Google LLC")],
);
if (dejavuDir === undefined) {
  problems.push("dejavu: the crate is not among the app's dependencies any more; update the DejaVu fonts entry");
} else {
  const assets = join(dejavuDir, "assets");
  const fonts = readdirSync(assets).find((name) => name.startsWith("dejavu-fonts"));
  const license = fonts ? join(assets, fonts, "LICENSE") : undefined;
  if (license === undefined || !existsSync(license)) problems.push(`dejavu: no font license under ${assets}`);
  else {
    addComponent(
      {
        name: "DejaVu fonts",
        version: /(\d+(?:\.\d+)+)/.exec(fonts ?? "")?.[1] ?? "",
        group: "data",
        license: "Bitstream-Vera",
        url: "https://dejavu-fonts.github.io/",
        note: "DejaVu Sans Bold draws the AI label on generated images.",
      },
      [readFileSync(license, "utf8")],
    );
  }
}

// ---- The installer (made by `tauri build` with NSIS; the version is the one the Tauri CLI downloads)

addComponent(
  {
    name: "NSIS (Nullsoft Scriptable Install System)",
    group: "installer",
    license: "Zlib AND CPL-1.0",
    url: "https://nsis.sourceforge.io/",
    note: "The installer is compressed with its LZMA module, under CPL-1.0 with the exception at the end of the text.",
  },
  [readFileSync(join(root, "scripts", "notices", "nsis-COPYING.txt"), "utf8")],
);
addComponent(
  {
    name: "nsis_tauri_utils",
    group: "installer",
    license: "MIT",
    url: "https://github.com/tauri-apps/nsis-tauri-utils",
    note: "An NSIS plugin from Tauri; offered under MIT OR Apache-2.0.",
  },
  [mitStock("2019 - 2022 Tauri Programme within The Commons Conservancy")],
);

// ---- Write

if (problems.length > 0) {
  console.error(`The third-party notices were not written. Look at these first:\n  ${problems.join("\n  ")}`);
  process.exit(1);
}

const order: Group[] = ["app", "screen", "data", "installer"];
components.sort(
  (a, b) =>
    order.indexOf(a.group) - order.indexOf(b.group) ||
    a.name.localeCompare(b.name, "en") ||
    (a.version ?? "").localeCompare(b.version ?? "", "en", { numeric: true }),
);
// Number the texts in the order the list first uses them.
const renumbered = new Map<number, number>();
for (const component of components) {
  for (const index of component.texts) if (!renumbered.has(index)) renumbered.set(index, renumbered.size);
}
const orderedTexts = [...renumbered.keys()].map((index) => texts[index] ?? "");
for (const component of components) component.texts = component.texts.map((index) => renumbered.get(index) ?? -1);

const appVersion = (JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")) as { version: string })
  .version;

const jsonPath = join(outDir, "third-party-notices.json");
writeFileSync(jsonPath, `${JSON.stringify({ appVersion, components, texts: orderedTexts })}\n`);

const lines = [
  `Third-party notices for Maskette ${appVersion}`,
  "",
  "Maskette is free software under GPL-3.0-or-later (see LICENSE). This build of",
  "Maskette includes the components below, which others made, each under its own",
  "license. The license texts follow the list; [n] after a component is the",
  "number of its text.",
];
for (const group of order) {
  const members = components.filter((component) => component.group === group);
  if (members.length === 0) continue;
  lines.push("", "", GROUP_TITLES[group], "=".repeat(GROUP_TITLES[group].length), "");
  for (const component of members) {
    const numbers = component.texts.map((index) => `[${index + 1}]`).join(" ");
    lines.push(`${[component.name, component.version].filter(Boolean).join(" ")}  ${component.license}  ${numbers}`);
    lines.push(`    ${component.url}`);
    if (component.note) lines.push(`    ${component.note}`);
  }
}
lines.push("", "", "License texts", "=============");
for (const [index, text] of orderedTexts.entries()) {
  const users = components
    .filter((component) => component.texts.includes(index))
    .map((component) => [component.name, component.version].filter(Boolean).join(" "));
  lines.push("", "", `[${index + 1}] ${users.join(", ")}`, "-".repeat(78), "", text);
}
const txtPath = join(outDir, "third-party-notices.txt");
writeFileSync(txtPath, `${lines.join("\n")}\n`);

const count = (group: Group) => components.filter((component) => component.group === group).length;
console.log(
  `Third-party notices: ${count("app")} for the app, ${count("screen")} for the screen, ${count("data")} models and ` +
    `fonts, ${count("installer")} for the installer; ${orderedTexts.length} license texts.`,
);
console.log(
  `Wrote ${relative(root, jsonPath).replaceAll("\\", "/")} and ${relative(root, txtPath).replaceAll("\\", "/")}`,
);
