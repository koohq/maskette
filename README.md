# Maskette

**Maskette — 着ぐるみマスクのデザインツール**
**Maskette · Kigurumi 头壳设计工具**

[日本語](README.ja.md) · [简体中文](README.zh-CN.md) (how to install and use)

Maskette is a desktop app for designing kigurumi masks from character art.
Give it reference pictures of a character, and its design assistant helps
you work out a 2D design, then a finished-mask front view and four views.
Every image can be adjusted by hand in the built-in editor, and each result
is kept in the project's version history.

## Credits

Maskette is based on
[Visual Studio Kigurumi](https://github.com/ElysiaRealm0019/Visual-Studio-Kigurumi)
(VSK) by [ElysiaRealm0019](https://github.com/ElysiaRealm0019), which is
based on [KigCraft](https://kigcraft.com) by SeaRabbit / 海兔
([icyqwq](https://github.com/icyqwq)). Thanks to both projects.

Like them, Maskette is licensed under GPL-3.0-or-later. See
[LICENSE](LICENSE), which also records what Maskette changed.

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

Other scripts:

| Script | What it does |
| --- | --- |
| `pnpm typecheck` | Type-check the Node scripts in `scripts/` and every workspace package, tests included |
| `pnpm test` | Run the frontend tests (Vitest) |
| `pnpm build` | Build the frontend into `frontend/dist` |
| `pnpm cargo:check` | `cargo check` the Tauri crate |
| `pnpm bundle` | Build the Windows installer, NSIS `-setup.exe` (`tauri build`), and copy it to `release/` |
| `pnpm release:copy` | Copy the latest built installer to `release/` again (Cargo may build outside the repository) |
| `pnpm lint` | Biome check |

Layout:

- `frontend/` — the UI and the image editor (Vite, React, pixi.js, konva,
  onnxruntime-web, MediaPipe)
- `src-tauri/` — the desktop shell and the local API, in Rust (Tauri, axum)
