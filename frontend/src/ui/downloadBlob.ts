/** How long a download's `blob:` URL stays alive after the click. */
const DOWNLOAD_URL_LIFETIME_MS = 60_000;

/** Saves `blob` as a download named `fileName`. */
export function downloadBlob(blob: Blob, fileName: string) {
  const objectUrl = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = objectUrl;
  link.download = fileName;
  link.rel = "noopener";
  document.body.appendChild(link);
  link.click();
  link.remove();
  // WebKit webviews (WebKitGTK on Linux) start the download after the click returns; revoking the URL right away
  // left them nothing to save.
  window.setTimeout(() => URL.revokeObjectURL(objectUrl), DOWNLOAD_URL_LIFETIME_MS);
}

/** `name` with the characters Windows does not allow in a file name replaced, or `fallback` when nothing is left. */
export function safeFileName(name: string, fallback: string): string {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: control characters are not allowed in file names either.
  const cleaned = name.replace(/[\\/:*?"<>|\u0000-\u001f]/g, "_").trim();
  return cleaned.replace(/[. ]+$/, "") || fallback;
}
