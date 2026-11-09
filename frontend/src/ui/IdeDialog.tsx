import { type FormEvent, type ReactNode, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useModalFocus } from "./useModalFocus";

/**
 * In-app replacements for window.confirm / window.prompt. Native dialogs are silently suppressed in some
 * embedded browsers and webviews, which made actions behind them look broken.
 *
 * `onConfirm` may throw; the message is shown in the dialog and it stays open so the user can retry or cancel.
 *
 * Keyboard: focus moves into the dialog when it opens (to the field of a prompt, else the dialog itself), Tab stays
 * inside, Escape cancels, and focus goes back to the opening button when it closes.
 */
type DialogBaseProps = {
  title: string;
  confirmLabel: string;
  danger?: boolean | undefined;
  onCancel: () => void;
  errorMessage?: ((error: unknown) => string) | undefined;
};

export function ConfirmDialog({
  title,
  message,
  confirmLabel,
  danger,
  onCancel,
  onConfirm,
  errorMessage,
}: DialogBaseProps & { message: string; onConfirm: () => Promise<void> | void }) {
  return (
    <DialogShell
      confirmLabel={confirmLabel}
      danger={danger}
      errorMessage={errorMessage}
      onCancel={onCancel}
      onSubmit={onConfirm}
      title={title}
    >
      <p className="text-sm text-(--ide-text-muted)">{message}</p>
    </DialogShell>
  );
}

export function PromptDialog({
  title,
  initialValue = "",
  placeholder,
  maxLength,
  allowEmpty = false,
  confirmLabel,
  onCancel,
  onConfirm,
  errorMessage,
}: DialogBaseProps & {
  initialValue?: string;
  placeholder?: string;
  maxLength?: number;
  /** Let the user confirm an empty value (a new project without a name is "untitled"). */
  allowEmpty?: boolean;
  onConfirm: (value: string) => Promise<void> | void;
}) {
  const [value, setValue] = useState(initialValue);
  return (
    <DialogShell
      confirmLabel={confirmLabel}
      disabled={!allowEmpty && !value.trim()}
      errorMessage={errorMessage}
      onCancel={onCancel}
      onSubmit={() => onConfirm(value.trim())}
      title={title}
    >
      <input
        aria-label={title}
        className="ide-input"
        data-autofocus
        maxLength={maxLength}
        onChange={(event) => setValue(event.target.value)}
        onFocus={(event) => event.target.select()}
        placeholder={placeholder}
        value={value}
      />
    </DialogShell>
  );
}

function DialogShell({
  title,
  confirmLabel,
  danger,
  disabled,
  onCancel,
  onSubmit,
  errorMessage,
  children,
}: DialogBaseProps & { disabled?: boolean; onSubmit: () => Promise<void> | void; children: ReactNode }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement | null>(null);
  useModalFocus(dialogRef);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || disabled) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit();
    } catch (caught) {
      setError(errorMessage ? errorMessage(caught) : caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: a pointer-only shortcut for Cancel; keyboard users have Escape and the Cancel button, and focus is already inside the dialog (useModalFocus)
    <div className="ide-dialog-backdrop" onClick={busy ? undefined : onCancel} role="presentation">
      <form
        aria-labelledby={titleId}
        aria-modal="true"
        className="ide-dialog"
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape" && !busy) onCancel();
        }}
        onSubmit={(event) => void submit(event)}
        ref={dialogRef}
        role="dialog"
        tabIndex={-1}
      >
        <h2 className="text-base font-semibold" id={titleId}>
          {title}
        </h2>
        {children}
        {error ? (
          <p className="text-sm text-(--ide-danger)" role="alert">
            {error}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <button className="ide-button" disabled={busy} onClick={onCancel} type="button">
            {t("workspace.cancel")}
          </button>
          <button
            className={`ide-button ${danger ? "ide-button-danger" : "ide-button-primary"}`}
            disabled={busy || disabled}
            type="submit"
          >
            {confirmLabel}
          </button>
        </div>
      </form>
    </div>
  );
}
