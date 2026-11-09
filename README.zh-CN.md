# Maskette — Kigurumi 头壳设计工具

Maskette 是一款根据角色立绘设计 Kigurumi 头壳的桌面应用。
提供参考图后，你可以和 AI 助手一起讨论，逐步做出设定图、成品头壳的正面图和四面图。
所有图片都能在内置编辑器里手动调整，每次结果都会保存在项目的版本历史中。

[English](README.md) · [日本語](README.ja.md)

## 运行环境

- Windows 10 / 11（64 位）
- 任意一种 AI 接入方式（见下文“接入 AI”）

暂无 macOS 版。

## 安装

1. 从发布页面下载 `Maskette_<版本>_x64-setup.exe` 并打开。
2. 如果出现“Windows 已保护你的电脑”，请点击“更多信息”→“仍要运行”。
   Maskette 目前还没有代码签名，所以会出现这个提示。
3. 按照向导安装。不需要管理员权限（只安装给当前用户）。
   如果电脑上没有 WebView2（微软用于显示界面的组件），安装程序会自动安装，此时需要联网。

有新版本时，用同样的方式运行新的 `setup.exe` 即可覆盖安装，已有的项目会保留。

## 接入 AI

首次使用时，请在界面上方的“设置”里选择 AI 的接入方式并填写 API Key。

- 最简单的方式是填写一个 [OpenRouter](https://openrouter.ai/) 的 API Key。对话、参考图分析、生图这三项都能用它完成，按用量向 OpenRouter 付费。
- API Key 保存在 Windows 凭据管理器中，不会写入文件。
- 也可以调用已用 ChatGPT 或 Claude 个人订阅登录的 CLI（Codex CLI、Claude Code）。请仅在自己的电脑上自己使用。

## 数据位置

- 项目和图片保存在 `%APPDATA%\app.maskette`。
- 在项目列表的菜单中选择“导出 .vkp”，可以把项目导出为一个文件，在另一台电脑上导入。

## 卸载

在 Windows 的“设置”→“应用”中卸载 Maskette。
如果也要删除项目，请在卸载界面勾选删除应用数据的选项。
如果也要删除 API Key，请在卸载前在 Maskette 的“设置”中恢复默认。

## 许可证

Maskette 基于 ElysiaRealm0019 的 [Visual Studio Kigurumi](https://github.com/ElysiaRealm0019/Visual-Studio-Kigurumi)（VSK），
VSK 又基于 SeaRabbit / 海兔（icyqwq）的 [KigCraft](https://kigcraft.com)。
许可证为 GPL-3.0-or-later，任何人都可以自由使用、修改和分发。
详见 [LICENSE](LICENSE)。源代码就在本仓库中。
