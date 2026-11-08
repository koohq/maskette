import { IconChevronLeft, IconChevronRight, IconRefresh } from "@tabler/icons-react";
import { type CSSProperties, type FormEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { translateLiteralString } from "../../../i18n/literalTranslations";
import { ActionIcon, Group, Stack, Text, Tooltip } from "../../../ui/mui";

export type ParameterSliderProps = {
  dataTestId: string;
  label: string;
  max: number;
  min: number;
  precision?: number | undefined;
  step: number;
  value: number;
  defaultValue?: number | undefined;
  debugValue?: string | undefined;
  debugValueFormatter?: ((value: number) => string) | undefined;
  onChange: (value: number) => void;
  onInteractionEnd?: (() => void) | undefined;
  onInteractionStart?: (() => void) | undefined;
  onReset: () => void;
};

function clampValue(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function roundToStep(value: number, step: number, precision: number) {
  const stepped = Math.round(value / step) * step;
  const factor = 10 ** Math.max(precision, 0);
  return Math.round(stepped * factor) / factor;
}

export function ParameterSlider({
  dataTestId,
  defaultValue = 0,
  debugValue,
  debugValueFormatter,
  label,
  max,
  min,
  precision = 1,
  step,
  value,
  onChange,
  onInteractionEnd,
  onInteractionStart,
  onReset,
}: ParameterSliderProps) {
  const { i18n } = useTranslation();
  const [localValue, setLocalValue] = useState(value);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const isInteractingRef = useRef(false);
  const lastSubmittedValueRef = useRef(value);
  const queuedValueRef = useRef(value);
  const changeFrameRef = useRef<number | null>(null);
  const onChangeRef = useRef(onChange);
  const finishInteractionRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-normalizes the shown value when the range or step changes; normalizeSliderValue is recreated each render from those props
  useEffect(() => {
    if (isInteractingRef.current) return;
    const nextValue = normalizeSliderValue(value);
    lastSubmittedValueRef.current = nextValue;
    queuedValueRef.current = nextValue;
    setLocalValue(nextValue);
  }, [dataTestId, max, min, precision, step, value]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: unmount-only cleanup; both helpers only touch refs and listeners
  useEffect(
    () => () => {
      removeFinishListeners();
      cancelQueuedChange();
    },
    [],
  );

  const visualValue = normalizeSliderValue(localValue);
  const progress = max > min ? ((visualValue - min) / (max - min)) * 100 : 0;
  const hasChanged = Math.abs(visualValue - defaultValue) > 10 ** -Math.max(precision, 1);
  const resolvedDebugValue = debugValueFormatter?.(visualValue) ?? debugValue;

  function normalizeSliderValue(nextValue: number) {
    return clampValue(roundToStep(nextValue, step, precision), min, max);
  }

  function submitQueuedValue() {
    changeFrameRef.current = null;
    const nextValue = queuedValueRef.current;
    if (nextValue === lastSubmittedValueRef.current) return;

    lastSubmittedValueRef.current = nextValue;
    onChangeRef.current(nextValue);
  }

  function queueValueChange(nextValue: number) {
    queuedValueRef.current = nextValue;
    if (changeFrameRef.current !== null) return;

    changeFrameRef.current = window.requestAnimationFrame(submitQueuedValue);
  }

  function flushQueuedChange() {
    if (changeFrameRef.current !== null) {
      window.cancelAnimationFrame(changeFrameRef.current);
      changeFrameRef.current = null;
    }
    submitQueuedValue();
  }

  function cancelQueuedChange() {
    if (changeFrameRef.current === null) return;

    window.cancelAnimationFrame(changeFrameRef.current);
    changeFrameRef.current = null;
  }

  function emitValue(nextValue: number, options: { flush?: boolean } = {}) {
    const normalizedValue = normalizeSliderValue(nextValue);
    setLocalValue(normalizedValue);
    queueValueChange(normalizedValue);
    if (options.flush) {
      flushQueuedChange();
    }
  }

  function beginInteraction() {
    if (!isInteractingRef.current) {
      isInteractingRef.current = true;
      onInteractionStart?.();
      addFinishListeners();
    }
  }

  function finishInteraction() {
    if (!isInteractingRef.current) return;

    const inputValue = Number(inputRef.current?.value);
    if (Number.isFinite(inputValue)) {
      emitValue(inputValue, { flush: true });
    } else {
      flushQueuedChange();
    }

    isInteractingRef.current = false;
    removeFinishListeners();
    onInteractionEnd?.();
  }

  function addFinishListeners() {
    if (finishInteractionRef.current) return;

    finishInteractionRef.current = finishInteraction;
    window.addEventListener("pointerup", finishInteraction, { capture: true });
    window.addEventListener("pointercancel", finishInteraction, { capture: true });
    window.addEventListener("mouseup", finishInteraction, { capture: true });
    window.addEventListener("touchend", finishInteraction, { capture: true });
    window.addEventListener("blur", finishInteraction);
  }

  function removeFinishListeners() {
    const finish = finishInteractionRef.current;
    if (!finish) return;

    window.removeEventListener("pointerup", finish, true);
    window.removeEventListener("pointercancel", finish, true);
    window.removeEventListener("mouseup", finish, true);
    window.removeEventListener("touchend", finish, true);
    window.removeEventListener("blur", finish);
    finishInteractionRef.current = null;
  }

  function handleInput(event: FormEvent<HTMLInputElement>) {
    beginInteraction();
    emitValue(Number(event.currentTarget.value));
  }

  function stepBy(direction: -1 | 1) {
    isInteractingRef.current = false;
    removeFinishListeners();
    emitValue(visualValue + step * direction, { flush: true });
    onInteractionEnd?.();
  }

  return (
    <Stack gap={0.75}>
      <Group justify="space-between" wrap="nowrap">
        <Text c={hasChanged ? "cyan" : "white"} fw={600} size="sm">
          {label}
        </Text>
        <Text c={hasChanged ? "cyan.3" : "dimmed"} fw={hasChanged ? 700 : 400} size="sm">
          {visualValue.toFixed(precision)}
          {resolvedDebugValue ? ` (${resolvedDebugValue})` : ""}
        </Text>
      </Group>
      <div style={{ alignItems: "center", display: "flex", flexWrap: "nowrap", gap: 8 }}>
        <Tooltip label={`${label}减少`}>
          <ActionIcon
            aria-label={`${label}减少`}
            disabled={visualValue <= min}
            onClick={() => stepBy(-1)}
            size="sm"
            variant="light"
          >
            <IconChevronLeft size={16} />
          </ActionIcon>
        </Tooltip>
        <div className="parameter-slider-shell" data-testid={dataTestId}>
          <input
            ref={inputRef}
            aria-label={translateLiteralString(label, i18n.language)}
            aria-valuemax={max}
            aria-valuemin={min}
            aria-valuenow={visualValue}
            className="parameter-slider-input"
            max={max}
            min={min}
            onBlur={finishInteraction}
            onChange={handleInput}
            onInput={handleInput}
            onMouseDown={beginInteraction}
            onPointerDown={beginInteraction}
            onTouchStart={beginInteraction}
            step={step}
            style={{ "--parameter-slider-progress": `${progress}%` } as CSSProperties}
            type="range"
            value={visualValue}
          />
        </div>
        <Tooltip label={`${label}增加`}>
          <ActionIcon
            aria-label={`${label}增加`}
            disabled={visualValue >= max}
            onClick={() => stepBy(1)}
            size="sm"
            variant="light"
          >
            <IconChevronRight size={16} />
          </ActionIcon>
        </Tooltip>
        <Tooltip label={`重置${label}`}>
          <ActionIcon
            aria-label={`重置${label}`}
            data-testid={`${dataTestId}-reset`}
            onClick={() => {
              isInteractingRef.current = false;
              removeFinishListeners();
              onInteractionEnd?.();
              onReset();
            }}
            size="sm"
            variant="subtle"
          >
            <IconRefresh size={14} />
          </ActionIcon>
        </Tooltip>
      </div>
    </Stack>
  );
}
