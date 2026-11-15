---
version: "beta"
name: "Maskette IDE"
description: "An IDE-style workspace for designing kigurumi head shells. Its default colours come from the Maskette mascot: mandarin hair on warm greys."
colors:
  mascot-mandarin: "#F59A4A"
  mascot-green: "#3FBF80"
  dark-accent: "#F59A4A"
  dark-background: "#1A1817"
  light-accent: "#B54D0F"
  light-background: "#F7F5F2"
typography:
  ui:
    fontFamily: Inter, PingFang SC, Microsoft YaHei, system-ui
    fontSize: 13px
---

## Overview

The app looks like a code editor: a title bar, an activity bar with tools, an explorer, a tabbed canvas, the assistant panel and a status bar. It should feel like a precise maker tool, not a dashboard. Images, masks and landmarks must stay visually exact; chrome stays quiet so the artwork stands out.

## Colors

The default palette comes from the logo: the mascot's mandarin hair as the accent, on warm greys. Users can pick another palette in Settings (Colors); light or dark is a separate switch in the header. Keep the palettes few and each with its own reason; a new one should differ from the others in hue or contrast at a glance (a yellow "star" palette was dropped because it looked like `mikan` and `contrast`).

| Palette | Accent | Background | Notes |
| --- | --- | --- | --- |
| `mikan` (default) | mandarin `#F59A4A` / `#B54D0F` | warm grey | from the mascot's hair |
| `sakura` | pink `#F48FB1` / `#B8336A` | warm grey | from the mascot's blush; danger leans vermilion so it does not read as the accent |
| `wakaba` | green `#3FBF80` / `#1C7F4F` | warm grey | from the mascot's eyes; success turns blue so it does not read as the accent |
| `ai` | blue `#2E95F8` / `#116CED` | navy | the colours the app had before its own logo (light-mode faint and status colours darkened to meet the contrast rule below) |
| `sumi` | grey `#D4D4D4` / `#3A3A3A` | neutral grey | monochrome; only the status colours carry hue, so the artwork's colours stand out |
| `contrast` | mandarin `#FFA552` / `#8F3500` | near-black / white | high contrast for low vision: strongest text and borders; told apart from `mikan` by contrast, not hue |

`app/theme.ts` (`idePalettes`) is the single source of every colour, per palette and mode. The provider writes the chosen set onto `<html>` as the `--ide-*` CSS variables and hands the same values to MUI. `src/styles/index.css` repeats only the default palette so the first paint has colours; `theme.test.ts` fails if the two drift. Use the variables instead of hard-coded colours.

Default palette (`mikan`):

| Token | Dark | Light | Use |
| --- | --- | --- | --- |
| `accent` | `#F59A4A` | `#B54D0F` | primary buttons, active tabs, focus, sliders |
| `accent-contrast` | `#2A1404` | `#FFFFFF` | text on accent (dark brown on mandarin in dark mode for contrast) |
| `bg` | `#1A1817` | `#F7F5F2` | window background |
| `panel` / `panel-raised` | `#211F1D` / `#2A2725` | `#FFFFFF` / `#F0EDE8` | side panels, cards, menus |
| `border` / `border-strong` | `#33302D` / `#46413D` | `#E3DED6` / `#CFC8BE` | dividers and outlines |
| `text` / `text-muted` / `text-faint` | `#EEE9E4` / `#A39C96` / `#716A64` | `#2A211B` / `#6B6158` / `#8C8278` | text levels |
| `danger` / `success` / `warning` | `#F26D6D` / `#43C38C` / `#E5B454` | `#C93C3C` / `#1B7A52` / `#A86B12` | status only |

In every palette, body text, muted text and text on the accent meet WCAG AA on their backgrounds, status colours read as text, and `text-faint` reaches 3:1 (non-essential hints only); `theme.test.ts` checks all of this. To add a palette, add its id to `PALETTE_IDS`, its colours to `idePalettes` (both modes, every token), and its name to `settings.palettes` in each locale, then check the contrast.

## Typography

Use the system UI stack (`uiFontFamily` in `theme.ts`) at 13px. Keep letter spacing at `0`. In compact controls, reduce font size before letting text wrap.

## Layout

- Desktop: activity bar, explorer and tool panels on the left, tabbed canvas in the middle, assistant on the right, status bar at the bottom.
- Use CSS Grid and Flex with explicit min/max widths for panels; prevent horizontal overflow on narrow screens.
- Avoid nested cards. Use full panels, repeated item cards or tool surfaces.

## Components

- Buttons: `ide-button`, with `ide-button-primary` for the main action of a view. One primary action per area.
- Icon buttons: `ide-icon-button` with Tabler icons and a tooltip/`aria-label`. Do not use emoji as icons.
- Badges: `ide-badge` with `data-tone="accent" | "success"` for short states.
- Inputs: label above the field, `ide-input`, accent border on focus.

## Rules

- Do not introduce a second visual system for new pages or tools; reuse the `ide-*` classes and variables.
- Do not use pure black, neon colours, decorative blobs or heavy gradients.
- Keep image canvases, crop previews, masks and landmarks visually precise.
- Preserve existing workflow state, upload behaviour, editor tools, landmark detection and model assets.
