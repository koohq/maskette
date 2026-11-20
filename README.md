# Maskette

**Maskette — 着ぐるみマスクのデザインツール**
**Maskette · Kigurumi 头壳设计工具**

[日本語](README.ja.md) · [简体中文](README.zh-CN.md)

Maskette is a desktop app for designing kigurumi masks from character art.
Give it reference pictures of a character, and its design assistant helps
you work out a character design, then a head shell front view, a four-view
sheet (front, three-quarter, side and back), an eye close-up, and design
notes that put the shapes and colours into words.
Every image can be adjusted by hand in the built-in editor, and each result
is kept in the project's version history.

## System requirements

- Windows 10 / 11 (64-bit)
- One of these for the AI (see “Setting up the AI”):
  - An API key for an AI service (such as OpenRouter)
  - Codex CLI or Claude Code, signed in with a paid ChatGPT or Claude plan

There is no macOS version yet.

## Installing

1. Download `Maskette_<version>_x64-setup.exe` from the releases page and run it.
2. If you see “Windows protected your PC”, choose “More info” → “Run anyway”.
   Maskette is not code-signed yet, so Windows shows this warning.
3. Follow the installer. You do not need administrator rights: Maskette is
   installed for your Windows user only.
   If the PC does not have the WebView2 runtime (the Microsoft component that
   draws the app's screens), the installer adds it, which needs an internet
   connection.

When a new version is out, Maskette says so above the project list when it
starts. Press “Update”: Maskette closes, installs the new version and opens
again when it is done. You can also check from “App updates” in Settings,
where you can turn off the check at startup. Your projects and settings stay
as they are.

If the app cannot update itself, run the new `setup.exe` the same way. When it says “Already
Installed”, press “Next”. The uninstaller then opens: leave “Delete the
application data” unticked, press “Uninstall”, and press “Close” when it
finishes. You are back in the installer, so carry on installing. Your
projects stay as they are.

The first time you start Maskette, the project list has one sample project.
You can open it in the editor or export it without setting up the AI. You
can delete it if you do not need it; a deleted sample does not come back.

## Setting up the AI

First open Settings (the gear icon at the top right), choose the AI service
to use, and enter its API key.

- The simplest way is an [OpenRouter](https://openrouter.ai/) API key. With
  the default settings, enter it as the “OpenRouter API key” under “Chat
  assistant LLM” and save; reference analysis and image generation then use
  the same key. You pay OpenRouter for what you use.
- API keys are stored in Windows Credential Manager, not in a file.
- You can also use a CLI signed in with a paid ChatGPT or Claude plan. Codex
  CLI works for the chat, reference analysis and image generation; Claude
  Code works for the chat and reference analysis. To use only a paid ChatGPT
  plan, choose “Codex CLI” in “Chat assistant LLM”, “Reference analysis LLM”
  and “Image backend” (no API key needed). Respect each service's terms of
  use, and use it only on your own PC, for yourself.

## When the eyes do not match the art

The generated head shell front view sometimes gets the eyes wrong: the irises
come out more vivid than in the art, or eyes narrowed in a smile in the art
come out open. You can then redraw only the eyes with “Local gen” in the
editor.

1. In “Images” on the left, open a version of the front view, then choose
   “Local gen” at the far left.
2. With the brush, paint both eyes as one connected area, including the upper
   and lower lashes and the outer corners. Do not paint the brows or the
   nose. Painting both eyes together, a little generously, gives a cleaner
   result than painting each eye small.
3. In “Describe the change”, write what to change and what to keep.
   - To match the iris colour to the art: “Change only the iris colour to
     match the character art in the reference (no more vivid than the art).
     Keep the pupil shape, eye size, position, expression, lashes and
     highlights.”
   - To make narrowed, smiling eyes: “Make the eyes narrowed, smiling eyes
     like the character art in the reference, with the upper lids covering
     the top of the irises. Curve the upper lids in an arc and lift the lower
     lids slightly. Keep the pupil shape, iris colour, eye position and lash
     colour.”
     “Narrow the eyes” alone tends to give half-closed eyes with straight
     upper lids, so describe the arc of the upper lids and the lift of the
     lower lids too.
4. Under “Reference image (optional)”, use “Choose an existing reference” to
   pick the character's original art (front), then press “Submit to AI to
   redo only this area”. Do not pick a picture cropped to the eyes alone
   (it gets pasted in as drawn, in the art's style).

The redrawn picture appears as a new version in the assistant panel on the
right. The iris colour may not match the art in one go. Then run Local gen
again, or add the colour in words, such as “greyish green”. The reflections
in the eyes may also get smaller.

If you change the “Model” under “Image backend” in Settings, other
differences may show up, such as vertically long pupils or a grey shadow
under the whites of the eyes. Fix them the same way, writing what to change.

To fix one spot, use Local gen in the editor rather than asking the chat to
revise the front view. Nothing outside the painted area changes. A revision
in the chat redraws the whole picture, so besides what you asked for, the
background colour or the face's position and size may shift a little.

## Eye close-up

The eyes are small in the head shell front view, so their design can be hard
to read.

After you approve the head shell front view and the four-view sheet is made,
the assistant asks whether to make an eye close-up too. If you ask for it, it
redraws both eyes and brows of the approved front view, large and clear (one
image generation). You can also ask for it in the chat: “Make an eye
close-up.”

- The eye close-up is made from the approved front view. To change the eyes,
  first fix the front view and approve it again, then make the eye close-up
  again.
- The AI fills in details the small front view does not show (the shape of
  the highlights, the lash tips and so on). Compare it with the front view
  before you use it.

## Design notes

Pictures generated by AI can differ a little in the eyes or hairstyle from
one picture to the next. Design notes put into words the shapes and colours
the pictures alone may not get across.

After the eye close-up (or after you say you do not want one), the assistant
asks whether to write design notes too. If you ask for them, the AI used for
reference analysis reads the approved front view, the four-view sheet and
the eye close-up, and puts the shapes and colours of the eyes, brows, mouth,
skin, hair, ears and accessories into words (one analysis). You can also ask
for them in the chat: “Write design notes.”

- The design notes appear as a card in the chat. Use “Copy” to paste them
  into another app, or “Save as text” to save them as a file.
- Left and right are as seen when looking at the front view (the viewer's
  left and right). Fixed sentences are added on which picture comes first
  when the pictures disagree, and on colours looking different on screen and
  in real life.
- Tell the chat what is wrong, or what you would like to add (such as “I
  want the star hair clips to be removable”), and it rewrites the notes.
  Ask for them “in Japanese” or “in Chinese” to get them in that language.
- The design notes are the AI's reading of the pictures. Compare them with
  the pictures before you use them. If you approve the front view again,
  have the design notes rewritten too.

## Where your data is kept

- Projects and images are saved in `%APPDATA%\app.maskette`.
- In the project list, choose “Export .maskette” from a project's “…” menu
  to write the project to one file, and bring it into another PC with
  “Import project”. You can also import .vkp files exported from Visual
  Studio Kigurumi (.maskette files cannot be opened in Visual Studio
  Kigurumi).

## Uninstalling

Uninstall Maskette from Windows Settings → Apps.
To delete your projects too, tick “Delete the application data” in the
uninstaller.
To delete the saved API keys too, press “Reset all to defaults” in
Maskette's Settings before uninstalling (this also resets your other
settings).

## Credits

Maskette is based on
[Visual Studio Kigurumi](https://github.com/ElysiaRealm0019/Visual-Studio-Kigurumi)
(VSK) by [ElysiaRealm0019](https://github.com/ElysiaRealm0019), which is
based on [KigCraft](https://github.com/icyqwq/KigCraft) by SeaRabbit / 海兔
([icyqwq](https://github.com/icyqwq)). Thanks to both projects.

Like them, Maskette is licensed under GPL-3.0-or-later: anyone may use,
change and share it under the GPL's terms. See [LICENSE](LICENSE), which
also records what Maskette changed.

## Development

Requirements:

- Node.js 26 and pnpm. With [mise](https://mise.jdx.dev/), `mise install` sets up both from `mise.toml`; pnpm then switches itself to the version in `packageManager` (`package.json`)
- Rust (stable, 1.90 or later)
- The [Tauri prerequisites](https://tauri.app/start/prerequisites/) for
  your OS. On Windows: Microsoft C++ Build Tools ("Desktop development with
  C++") and WebView2.

```bash
pnpm install
pnpm dev            # opens the Maskette window (tauri dev)
```

`pnpm dev` keeps its data in the same folder as the installed app
(`%APPDATA%\app.maskette`). Debug builds use the folder in the
`MASKETTE_DATA_DIR` environment variable instead when it is set; a folder
that does not exist yet tries a first launch, the only time the sample
project (`src-tauri/sample/`) is added.

Other scripts:

| Script | What it does |
| --- | --- |
| `pnpm typecheck` | Type-check the Node scripts in `scripts/` and every workspace package, tests included |
| `pnpm test` | Run the frontend tests (Vitest) |
| `pnpm vrt` | Screenshot the main screens in every language at the default and the smallest window size, compare them with `frontend/vrt/__screenshots__`, and check that no text or control overflows its box (Playwright; the API is answered by fixtures). The first run needs `pnpm --filter maskette-frontend exec playwright install chromium` |
| `pnpm vrt:update` | Accept the current screenshots as the new baselines; look at the changed images before committing them |
| `pnpm build` | Build the frontend into `frontend/dist` |
| `pnpm cargo:check` | `cargo check` the Tauri crate |
| `pnpm bundle` | Build the Windows installer, NSIS `-setup.exe` (`tauri build`), sign it for the updater, and copy it to `release/` with the `latest.json` that installed copies read. Needs the signing key (below) |
| `pnpm bundle:local` | Build the installer without the updater's files, so no signing key is needed. For trying a build on your own PC |
| `pnpm release:copy` | Copy the latest built installer to `release/` again (Cargo may build outside the repository); `--notes <file>` puts release notes into `latest.json` |
| `pnpm lint` | Biome check |

Releasing an update: installed copies of Maskette read
`latest.json` from the latest GitHub release
(`plugins.updater.endpoints` in `src-tauri/tauri.conf.json`) and accept only
installers signed with the key whose public half is `plugins.updater.pubkey`.

1. Raise `version` in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`.
2. In the shell that builds, set `TAURI_SIGNING_PRIVATE_KEY` (the path to the
   private key, or its contents) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
   A `.env` file does not work for these.
3. `pnpm bundle`, then `pnpm release:copy --notes <file>` to add release
   notes.
4. Create a GitHub release tagged `v<version>` and attach the installer and
   `latest.json` from `release/`. The installer's URL in `latest.json`
   points at that tag.

Layout:

- `frontend/` — the UI and the image editor (Vite, React, pixi.js, konva,
  onnxruntime-web, MediaPipe)
- `src-tauri/` — the desktop shell and the local API, in Rust (Tauri, axum)
- `src-tauri/sample/` — the sample project, an unzipped `.maskette` file
