# Maskette

English | [日本語](README.ja.md)

**Maskette — 着ぐるみマスクのデザインツール**
**Maskette · Kigurumi 头壳设计工具**

Maskette is a desktop app for designing kigurumi head shells (masks). The
name plays on *mask* and *maquette*. It runs as a local
[Tauri](https://tauri.app) app: the editor UI is TypeScript/React, and the
backend is being rebuilt in Rust.

## Lineage

```
KigCraft (SeaRabbit / 海兔, https://kigcraft.com)
  └─ Visual Studio Kigurumi (VSK)
       └─ Maskette
```

Maskette started from the frontend of
[Visual Studio Kigurumi](https://github.com/ElysiaRealm0019/Visual-Studio-Kigurumi)
(VSK), which is itself based on [KigCraft](https://kigcraft.com) by
SeaRabbit / 海兔. The conversational workflow, workspace and generation
pipeline come from KigCraft; VSK added switchable model backends, a settings
page, editor improvements and prompt tuning. Maskette is a separate
repository, not a GitHub fork, and does not merge from either project; it
takes ideas from them where they fit.

## Development

Requirements:

- Node.js 26 and pnpm (see `.node-version` and `mise.toml`)
- Rust (stable, 1.90 or later)
- The [Tauri prerequisites](https://tauri.app/start/prerequisites/) for
  your OS. On Windows: Microsoft C++ Build Tools ("Desktop development with
  C++") and WebView2.

```bash
pnpm install
pnpm dev            # opens the Maskette window (tauri dev)
```

To compare with the old Python backend while routers are being ported, run
it (from Visual Studio Kigurumi, on port 18000) and set
`MASKETTE_LEGACY_API_URL=http://127.0.0.1:18000` before `pnpm dev`; API
routes Maskette does not handle yet are then relayed to it.

Other scripts:

| Script | What it does |
| --- | --- |
| `pnpm typecheck` | Type-check every workspace package |
| `pnpm test` | Run the frontend tests (Vitest) |
| `pnpm build` | Build the frontend into `frontend/dist` |
| `pnpm cargo:check` | `cargo check` the Tauri crate |
| `pnpm bundle` | Build the desktop installer (`tauri build`) |
| `pnpm lint` | Biome check |

Layout:

- `frontend/` — the editor UI (Vite, React, pixi.js, konva,
  onnxruntime-web, MediaPipe)
- `src-tauri/` — the Tauri shell and, later, the Rust backend
- `docs/` — design notes and hand-over documents

## License

GPL-3.0-or-later. See [LICENSE](LICENSE), which also records the lineage
and what Maskette changed.
