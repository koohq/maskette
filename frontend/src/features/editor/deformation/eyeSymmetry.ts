// The eye-symmetry correction: measure a generated face's eye pair and, when it is uneven, rebuild it as one mirrored
// pair from the better eye's own pixels. Ported from VSK (`eyeSymmetry.ts`, GPL-3.0-or-later); the guards that leave a
// wink and two different iris colours alone are Maskette's.

import type { EyeRegion, LandmarkPoint, ManualLandmarks } from "./landmarks";

export type EyeSymmetrySide = "left" | "right";

/**
 * Everything needed to rebuild an uneven eye pair as one mirrored pair.
 *
 * All values are image-normalized, so one plan describes the correction for any resolution of the same artwork.
 */
export type EyeSymmetryPlan = {
  /** Face centreline the corrected pair is mirrored across. */
  axisX: number;
  /** Shared height both eye centres are moved to. */
  eyeY: number;
  /** Half the distance between the two corrected eye centres, kept from the detected pair. */
  halfSpacing: number;
  patchHalfWidth: number;
  patchHalfHeight: number;
  /** Eye whose pixels become the source of both corrected eyes; the other one is replaced by its mirror. */
  master: EyeSymmetrySide;
  /** Detected centre of the master eye: the patch is cropped here (in the master's own orientation). */
  masterCenter: LandmarkPoint;
  /** How uneven the detected pair was. */
  heightDelta: number;
  spacingDelta: number;
  sizeRatio: number;
};

/**
 * When an uneven pair counts as needing correction, and how the patch is cut.
 *
 * `heightDelta` is a fraction of the image height: on the 800x1100 front canvas it is about 7 px, roughly a tenth of an
 * eye's height, which is where an offset starts to read as a mistake instead of a style choice. The thresholds sit
 * above landmark-detection noise, so an already symmetric pair never produces a correction.
 */
export const eyeSymmetryThresholds = {
  heightDelta: 0.006,
  /** Off-centre pair, relative to the distance between the eyes. */
  spacingDelta: 0.15,
  /** Relative difference between the two detected eye regions. */
  sizeRatio: 0.2,
  /**
   * An eye less than this share as open as the other (contour height over width) is closed: a wink or a one-sided
   * expression the design means, which a mirrored copy of the open eye would erase. Measured on a generated front view,
   * open eyes read 0.7-0.8 and a drawn closed eye 0.19.
   */
  closedEyeShare: 0.45,
  /** Patch size around the master eye's detected region: enough to cover the lashes and the lid edge. */
  patchScale: 1.35,
  /** The patch never reaches past this share of the distance to the centreline, so the two patches cannot overlap. */
  patchShareOfSpacing: 0.92,
  /** Fallback patch half sizes, relative to the distance between the eyes, when no eye region was detected. */
  fallbackPatchShare: 0.34,
  /** Outer share of the patch that fades into the original pixels, so no hard seam is left behind. */
  feather: 0.35,
  /**
   * How far above and below its centre an eye's lashes and lid line reach, relative to the eye region's half height.
   * The original eye under each patch sits half the height difference away from the levelled one; unless its lid line
   * falls inside the patch's unfaded core, it shows through the fade as a faint second lid.
   */
  lashReach: 1.4,
} as const;

export type EyeSymmetryConfidence = {
  left: number;
  right: number;
};

/** How open each eye is drawn, from its contour (`eyeOpenness` in the landmark mapping). */
export type EyeOpenness = {
  left: number;
  right: number;
};

/** How much more the detector has to trust one eye before it wins as the master over the larger one. */
const confidenceAdvantage = 0.08;

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function round(value: number) {
  return Number(value.toFixed(4));
}

function regionArea(region: EyeRegion) {
  return region.radiusX * region.radiusY;
}

/** Whether one eye is drawn closed while the other is open: a wink, which is kept. */
export function isOneEyeClosed(openness: EyeOpenness): boolean {
  const open = Math.max(openness.left, openness.right);
  if (!(open > 0)) return false;
  return Math.min(openness.left, openness.right) < open * eyeSymmetryThresholds.closedEyeShare;
}

/**
 * Reads one detected eye pair and returns the correction that makes it mirror-symmetric, or null when the pair is
 * already level, centred and equally sized within tolerance, or when one eye is closed on purpose.
 *
 * The master eye is the one a human would trust more: the more confidently detected one, or, without a clear
 * difference in confidence, the larger region - the other eye is usually the foreshortened or partly hidden one.
 */
export function analyzeEyeSymmetry(
  landmarks: ManualLandmarks,
  confidence?: EyeSymmetryConfidence,
  openness?: EyeOpenness,
): EyeSymmetryPlan | null {
  const { leftEye, rightEye } = landmarks;
  const eyeDistance = Math.abs(rightEye.x - leftEye.x);
  if (!(eyeDistance > 0.02)) return null;
  if (openness && isOneEyeClosed(openness)) return null;

  const heightDelta = Math.abs(leftEye.y - rightEye.y);
  const eyeCenterX = (leftEye.x + rightEye.x) / 2;
  const axisX = faceCenterline(landmarks, eyeCenterX);
  const spacingDelta = Math.abs(Math.abs(axisX - leftEye.x) - Math.abs(rightEye.x - axisX)) / eyeDistance;
  const sizeRatio = eyeRegionSizeRatio(landmarks);

  const needsCorrection =
    heightDelta > eyeSymmetryThresholds.heightDelta ||
    spacingDelta > eyeSymmetryThresholds.spacingDelta ||
    sizeRatio > eyeSymmetryThresholds.sizeRatio;
  if (!needsCorrection) return null;

  const master = pickMasterEye(landmarks, confidence);
  const masterCenter = master === "left" ? leftEye : rightEye;
  const masterRegion = landmarks.eyeRegions?.[master];
  const masterHalfHeight = masterRegion ? Math.max(masterRegion.radiusTopY, masterRegion.radiusBottomY) : 0;
  const halfSpacing = eyeDistance / 2;
  const maxPatchHalfWidth = halfSpacing * eyeSymmetryThresholds.patchShareOfSpacing;
  const fallbackHalfSize = eyeDistance * eyeSymmetryThresholds.fallbackPatchShare;
  const patchHalfWidth = Math.min(
    maxPatchHalfWidth,
    Math.max(masterRegion ? masterRegion.radiusX * eyeSymmetryThresholds.patchScale : 0, fallbackHalfSize),
  );
  const patchHalfHeight = Math.max(
    masterHalfHeight * eyeSymmetryThresholds.patchScale,
    fallbackHalfSize,
    // The unfaded core has to cover the original eye's lid line, which sits half the height difference away;
    // otherwise the old lid shows through the fade.
    (heightDelta / 2 + masterHalfHeight * eyeSymmetryThresholds.lashReach) / (1 - eyeSymmetryThresholds.feather),
  );

  return {
    axisX: round(axisX),
    eyeY: round((leftEye.y + rightEye.y) / 2),
    halfSpacing: round(halfSpacing),
    patchHalfWidth: round(Math.max(patchHalfWidth, 0.01)),
    patchHalfHeight: round(Math.max(patchHalfHeight, 0.01)),
    master,
    masterCenter: { x: masterCenter.x, y: masterCenter.y },
    heightDelta: round(heightDelta),
    spacingDelta: round(spacingDelta),
    sizeRatio: round(sizeRatio),
  };
}

/**
 * Face centreline from the landmarks an uneven eye pair cannot move: chin, mouth and jaw. The eye midpoint is only the
 * fallback, so a badly placed eye cannot drag the mirror axis with it.
 */
function faceCenterline(landmarks: ManualLandmarks, eyeCenterX: number) {
  const candidates = [
    landmarks.chin.x,
    landmarks.mouthCenter.x,
    (landmarks.jawLeft.x + landmarks.jawRight.x) / 2,
  ].filter((value) => Number.isFinite(value));
  const faceAxis = candidates.length
    ? candidates.reduce((sum, value) => sum + value, 0) / candidates.length
    : eyeCenterX;
  const low = Math.min(landmarks.leftEye.x, landmarks.rightEye.x);
  const high = Math.max(landmarks.leftEye.x, landmarks.rightEye.x);

  // A face axis outside the eye pair means the landmarks disagree with each other: use the pair's own centre.
  if (!(faceAxis > low && faceAxis < high)) return eyeCenterX;
  return faceAxis;
}

/** Relative difference between the two detected eye regions: how much one eye is drawn larger than the other. */
function eyeRegionSizeRatio(landmarks: ManualLandmarks) {
  const regions = landmarks.eyeRegions;
  if (!regions) return 0;

  const leftArea = regionArea(regions.left);
  const rightArea = regionArea(regions.right);
  const mean = (leftArea + rightArea) / 2;
  if (!(mean > 0)) return 0;
  return Math.abs(leftArea - rightArea) / mean;
}

function pickMasterEye(landmarks: ManualLandmarks, confidence?: EyeSymmetryConfidence): EyeSymmetrySide {
  if (confidence) {
    const advantage = confidence.left - confidence.right;
    if (Math.abs(advantage) > confidenceAdvantage) return advantage > 0 ? "left" : "right";
  }

  const regions = landmarks.eyeRegions;
  if (regions) {
    const leftArea = regionArea(regions.left);
    const rightArea = regionArea(regions.right);
    if (Math.abs(leftArea - rightArea) > Number.EPSILON) return leftArea > rightArea ? "left" : "right";
  }

  return "left";
}

/** The dominant hue of an eye's clearly coloured pixels, and how much of the sample they fill. */
export type IrisColour = {
  /** Degrees, 0-360; NaN when no pixel is clearly coloured. */
  hue: number;
  colouredShare: number;
};

/** Pixels below these count as white, grey, black or skin rather than iris colour. */
const irisMinSaturation = 0.3;
const irisMinValue = 0.2;
/** An eye counts as coloured when this share of its sample is. */
const irisMinColouredShare = 0.08;
/** Two coloured irises this far apart in hue are two different eye colours. */
const irisHueDifference = 40;

/** Reads the iris colour out of RGBA pixels sampled from the centre of one eye. */
export function irisColour(data: ArrayLike<number>): IrisColour {
  let sumX = 0;
  let sumY = 0;
  let coloured = 0;
  let total = 0;
  for (let index = 0; index + 3 < data.length; index += 4) {
    if ((data[index + 3] ?? 0) < 128) continue;
    total += 1;
    const red = (data[index] ?? 0) / 255;
    const green = (data[index + 1] ?? 0) / 255;
    const blue = (data[index + 2] ?? 0) / 255;
    const max = Math.max(red, green, blue);
    const min = Math.min(red, green, blue);
    const saturation = max === 0 ? 0 : (max - min) / max;
    if (saturation < irisMinSaturation || max < irisMinValue) continue;
    const hue = hueDegrees(red, green, blue, max, max - min);
    sumX += Math.cos((hue * Math.PI) / 180);
    sumY += Math.sin((hue * Math.PI) / 180);
    coloured += 1;
  }
  const hue = coloured ? ((Math.atan2(sumY, sumX) * 180) / Math.PI + 360) % 360 : Number.NaN;
  return { hue, colouredShare: total ? coloured / total : 0 };
}

function hueDegrees(red: number, green: number, blue: number, max: number, chroma: number) {
  if (chroma === 0) return 0;
  if (max === red) return (60 * ((green - blue) / chroma) + 360) % 360;
  if (max === green) return 60 * ((blue - red) / chroma) + 120;
  return 60 * ((red - green) / chroma) + 240;
}

/**
 * Whether the two eyes have different iris colours (heterochromia). Mirroring one eye onto the other would give both
 * the same colour, so such a pair is never corrected.
 */
export function irisColoursDiffer(left: IrisColour, right: IrisColour): boolean {
  const leftColoured = left.colouredShare >= irisMinColouredShare;
  const rightColoured = right.colouredShare >= irisMinColouredShare;
  if (leftColoured !== rightColoured) {
    // A coloured iris against a grey or dark one, unless the shares are too close to call.
    return Math.abs(left.colouredShare - right.colouredShare) >= 0.1;
  }
  if (!leftColoured) return false;
  const difference = Math.abs(left.hue - right.hue) % 360;
  return Math.min(difference, 360 - difference) > irisHueDifference;
}

export type EyeSymmetrySize = {
  height: number;
  width: number;
};

/** Creates a canvas already sized for the given pixels; injectable so the correction can be tested without a GPU canvas. */
export type EyeSymmetryCanvasFactory = (width: number, height: number) => HTMLCanvasElement;

export function createSizedCanvas(width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));
  return canvas;
}

export function requireContext(canvas: HTMLCanvasElement): CanvasRenderingContext2D {
  const context = canvas.getContext("2d");
  if (!context) throw new Error("2D canvas is unavailable for the eye symmetry correction");
  return context;
}

/**
 * Paints the corrected pair onto `context`: the master eye's pixels are copied to both eye positions, the second copy
 * mirrored across the face centreline, so the result is exactly symmetric while keeping the character's own eye art.
 *
 * `source` is drawn as the base layer, so the result is the whole picture, not just the eyes.
 */
export function drawEyeSymmetry(
  context: CanvasRenderingContext2D,
  source: CanvasImageSource,
  plan: EyeSymmetryPlan,
  size: EyeSymmetrySize,
  createCanvas: EyeSymmetryCanvasFactory = createSizedCanvas,
) {
  const width = Math.max(1, Math.round(size.width));
  const height = Math.max(1, Math.round(size.height));
  context.clearRect(0, 0, width, height);
  context.drawImage(source, 0, 0, width, height);

  const axisX = plan.axisX * width;
  const eyeY = plan.eyeY * height;
  const halfSpacing = plan.halfSpacing * width;
  const patchHalfWidth = Math.max(2, plan.patchHalfWidth * width);
  const patchHalfHeight = Math.max(2, plan.patchHalfHeight * height);
  const patchWidth = Math.max(1, Math.round(patchHalfWidth * 2));
  const patchHeight = Math.max(1, Math.round(patchHalfHeight * 2));
  const masterCenter = { x: plan.masterCenter.x * width, y: plan.masterCenter.y * height };
  const patch = createMasterPatch(source, masterCenter, patchWidth, patchHeight, createCanvas);

  for (const side of ["left", "right"] as const) {
    const targetX = axisX + (side === "left" ? -halfSpacing : halfSpacing);
    context.save();
    context.translate(targetX, eyeY);
    // The master's own side keeps the artwork as drawn; the other side is its mirror image.
    if (side !== plan.master) context.scale(-1, 1);
    context.drawImage(patch, -patchWidth / 2, -patchHeight / 2);
    context.restore();
  }
}

/** Renders the corrected image onto a new canvas; the caller owns the canvas. */
export function renderEyeSymmetry(
  source: CanvasImageSource,
  plan: EyeSymmetryPlan,
  size: EyeSymmetrySize,
  createCanvas: EyeSymmetryCanvasFactory = createSizedCanvas,
): HTMLCanvasElement {
  const canvas = createCanvas(size.width, size.height);
  drawEyeSymmetry(requireContext(canvas), source, plan, size, createCanvas);
  return canvas;
}

/** Crops the master eye and fades its edge, so pasting it leaves no visible ellipse on the artwork. */
function createMasterPatch(
  source: CanvasImageSource,
  masterCenter: LandmarkPoint,
  patchWidth: number,
  patchHeight: number,
  createCanvas: EyeSymmetryCanvasFactory,
): HTMLCanvasElement {
  const patch = createCanvas(patchWidth, patchHeight);
  const patchContext = requireContext(patch);
  patchContext.drawImage(
    source,
    Math.round(masterCenter.x - patchWidth / 2),
    Math.round(masterCenter.y - patchHeight / 2),
    patchWidth,
    patchHeight,
    0,
    0,
    patchWidth,
    patchHeight,
  );

  const mask = createFeatherMask(createCanvas, patchWidth, patchHeight);
  patchContext.save();
  patchContext.globalCompositeOperation = "destination-in";
  patchContext.drawImage(mask, 0, 0);
  patchContext.restore();
  return patch;
}

function createFeatherMask(createCanvas: EyeSymmetryCanvasFactory, width: number, height: number): HTMLCanvasElement {
  const mask = createCanvas(width, height);
  const context = requireContext(mask);
  const radius = width / 2;
  context.clearRect(0, 0, width, height);
  context.save();
  context.translate(width / 2, height / 2);
  context.scale(1, height / width);
  const gradient = context.createRadialGradient(0, 0, 0, 0, 0, radius);
  const solidStop = clamp(1 - eyeSymmetryThresholds.feather, 0, 1);
  gradient.addColorStop(0, "rgba(0, 0, 0, 1)");
  gradient.addColorStop(solidStop, "rgba(0, 0, 0, 1)");
  gradient.addColorStop(1, "rgba(0, 0, 0, 0)");
  context.fillStyle = gradient;
  context.beginPath();
  context.arc(0, 0, radius, 0, Math.PI * 2);
  context.fill();
  context.restore();
  return mask;
}
