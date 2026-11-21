# Security policy

## Reporting a vulnerability

Please do not report security problems in public issues. Report them
privately instead, from
[Report a vulnerability](https://github.com/koohq/maskette/security/advisories/new)
(the “Security” tab of this repository). You may write in English, Japanese
or Chinese.

Please include the Maskette version (Settings → App updates), what you did
and what happened. Do not include API keys or other secrets. If a key may
have leaked, revoke it at its service first.

You will get a reply as soon as possible. After a fix is released, the
report may be published as a security advisory.

## What to report

For example:

- Another program or a website using Maskette's local API, which listens on
  `127.0.0.1` and accepts only the app's own requests
- API keys leaving Windows Credential Manager: in a file, a log, an exported
  project or a request to a service other than the one the key is for
- Maskette installing an update without a valid signature
- A project file (`.maskette` or `.vkp`) that, when imported, writes outside
  Maskette's data folder or runs code

Problems in an AI service, Codex CLI or Claude Code themselves belong with
those services.

## Supported versions

Fixes go into the latest release only. Update from Settings → App updates.

## 日本語

セキュリティの問題は、公開の Issue ではなく、[Report a vulnerability](https://github.com/koohq/maskette/security/advisories/new)（このリポジトリの「Security」タブ）から非公開で知らせてください。日本語で書いてかまいません。Maskette のバージョン（設定の「アプリの更新」）と、何をしてどうなったかを書いてください。API キーなどの秘密の情報は書かないでください。修正は最新のバージョンにだけ入ります。

## 中文

安全问题请不要发在公开的 Issue 里，请通过 [Report a vulnerability](https://github.com/koohq/maskette/security/advisories/new)（本仓库的“Security”标签页）私下报告，可以用中文填写。请写明 Maskette 的版本（设置中的“应用更新”）、做了什么以及发生了什么。请不要附上 API Key 等机密信息。修复只会加入最新版本。
