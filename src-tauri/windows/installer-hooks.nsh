; Hooks for the NSIS installer Tauri builds (`bundle.windows.nsis.installerHooks` in tauri.conf.json).

; The welcome and finish pages show `installer-sidebar.bmp` fitted to the height of their image
; area, keeping its proportions. NSIS's default stretches it to fill the area, whose shape follows
; the language's dialog font (wider in Japanese and Chinese than in English), and that made the
; mascot's face look wide. What the image leaves on the right is the page's white; the image
; fades to white at its right edge so the two meet without a seam.
!define MUI_WELCOMEFINISHPAGE_BITMAP_STRETCH "AspectFitHeight"
