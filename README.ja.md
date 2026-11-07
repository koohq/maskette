# Maskette

[English](README.md) | 日本語

**Maskette — 着ぐるみマスクのデザインツール**
**Maskette · Kigurumi 头壳设计工具**

Maskette は、着ぐるみマスク（頭部シェル）をデザインするデスクトップアプリです。
名前は *mask*（マスク）と *maquette*（マケット、ひな形）の掛け言葉です。
[Tauri](https://tauri.app) で手元で動くアプリとして作っており、エディタの画面は
TypeScript/React、バックエンドは Rust で作り直しています。

英語版の README が正です。食い違うときは英語版に従ってください。

## 系譜

```
KigCraft（SeaRabbit / 海兔、https://kigcraft.com）
  └─ Visual Studio Kigurumi（VSK）
       └─ Maskette
```

Maskette は [Visual Studio Kigurumi](https://github.com/ElysiaRealm0019/Visual-Studio-Kigurumi)
（VSK）のフロントエンドから始まりました。VSK は SeaRabbit / 海兔 による
[KigCraft](https://kigcraft.com) をもとにしています。対話で進める流れ、
ワークスペース、生成の仕組みは KigCraft から来ており、VSK はそこに切り替え
可能なモデルのバックエンド、設定ページ、エディタの改良、プロンプトの調整を
加えました。Maskette は GitHub のフォークではない別のリポジトリで、どちらの
プロジェクトからも merge はせず、合うところをアイデアとして取り込みます。

## 開発

必要なもの：

- Node.js 26 と pnpm（`.node-version` と `mise.toml` を参照）
- Rust（stable、1.90 以上）
- OS ごとの [Tauri の前提条件](https://tauri.app/start/prerequisites/)。
  Windows では Microsoft C++ Build Tools（「C++ によるデスクトップ開発」）と
  WebView2。

```bash
pnpm install
pnpm dev            # Maskette の窓を開く（tauri dev）
```

ルーターを移している間に旧 Python のバックエンドと見比べるときは、それを
（Visual Studio Kigurumi から、ポート 18000 で）起動し、`pnpm dev` の前に
`MASKETTE_LEGACY_API_URL=http://127.0.0.1:18000` を設定します。Maskette が
まだ扱わない API は、そちらへ中継されます。

ほかのスクリプト：

| スクリプト | 内容 |
| --- | --- |
| `pnpm typecheck` | ワークスペースの全パッケージを型チェック |
| `pnpm test` | フロントエンドのテスト（Vitest） |
| `pnpm build` | フロントエンドを `frontend/dist` にビルド |
| `pnpm cargo:check` | Tauri のクレートを `cargo check` |
| `pnpm bundle` | デスクトップ用インストーラーを作る（`tauri build`） |
| `pnpm lint` | Biome のチェック |

構成：

- `frontend/` — エディタの画面（Vite、React、pixi.js、konva、
  onnxruntime-web、MediaPipe）
- `src-tauri/` — Tauri の外枠と、のちの Rust バックエンド
- `docs/` — 設計メモと引き継ぎ資料

## ライセンス

GPL-3.0-or-later。系譜と Maskette で変えたことも書いた [LICENSE](LICENSE)
を参照してください。
