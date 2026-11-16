// Runs the eye-symmetry correction on a generated version: load its pixels, find the eyes, and return corrected pixels
// when the pair is uneven. Ported from VSK (`eyeSymmetryCorrection.ts`, GPL-3.0-or-later), with the two-colour guard
// added (the on/off setting is in `autoFixPreferences.ts`).

import { isTurnaroundRole } from "../agent/agentApi";
import { detectAnimeLandmarks } from "../editor/deformation/animeLandmarkDetector";
import { eyeLandmarkConfidences } from "../editor/deformation/animeLandmarkMapping";
import {
  analyzeEyeSymmetry,
  createSizedCanvas,
  type EyeSymmetryCanvasFactory,
  type EyeSymmetryPlan,
  irisColour,
  irisColoursDiffer,
  renderEyeSymmetry,
  requireContext,
} from "../editor/deformation/eyeSymmetry";
import type { LandmarkPoint, ManualLandmarks } from "../editor/deformation/landmarks";

/**
 * Roles generated from the user's references or from an earlier version, which can therefore inherit an uneven eye
 * pair from them. Reference uploads are never touched.
 */
export function supportsEyeSymmetryCorrection(role: string | undefined): boolean {
  return role === "design" || role === "design_turnaround" || role === "front" || role === "turnaround";
}

/** The front view is the leftmost head of a four-view sheet; the detector is pinned to that panel. */
const turnaroundRoiWidthShare = 0.26;

/** The version endpoint takes 20 MiB; a very detailed sheet keeps its original pixels instead. */
const maxCorrectedBytes = 19 * 1024 * 1024;

/** Side of the small canvas each eye's centre is sampled into for its iris colour. */
const irisSampleSize = 32;
/** Share of the eye region, around its centre, that is read as the iris. */
const irisSampleShare = 0.6;

export type EyeSymmetryCorrection = {
  blob: Blob;
  height: number;
  plan: EyeSymmetryPlan;
  width: number;
};

/**
 * Measures the generated image's eye pair and, when it is uneven, returns the pixels of a corrected copy whose eyes are
 * one mirrored pair. Returns null whenever there is nothing to fix, the eyes cannot be located confidently enough to
 * rewrite pixels (a wrong correction is worse than the asymmetry), or the two eyes are meant to differ: a wink, or two
 * iris colours.
 *
 * The correction is deterministic and costs no image generation.
 */
export async function correctEyeSymmetry(options: {
  imageUrl: string;
  role?: string | undefined;
  signal?: AbortSignal | undefined;
  createCanvas?: EyeSymmetryCanvasFactory | undefined;
}): Promise<EyeSymmetryCorrection | null> {
  if (!supportsEyeSymmetryCorrection(options.role)) return null;
  const { signal } = options;
  const createCanvas = options.createCanvas ?? createSizedCanvas;
  const image = await loadImageElement(options.imageUrl, signal);
  const width = image.naturalWidth || image.width;
  const height = image.naturalHeight || image.height;
  if (width <= 1 || height <= 1) return null;

  const regionOfInterest = isTurnaroundRole(options.role)
    ? { height, width: width * turnaroundRoiWidthShare, x: 0, y: 0 }
    : undefined;
  // Strict single-face mode on purpose: it rejects a weak landmark fit instead of guessing.
  const detection = await detectAnimeLandmarks(image, signal, true, regionOfInterest);
  if (!detection || signal?.aborted) return null;

  const plan = analyzeEyeSymmetry(detection.controls, eyeLandmarkConfidences(detection.debug.points));
  if (!plan) return null;
  if (eyesHaveDifferentColours(image, detection.controls, { height, width }, createCanvas)) return null;

  const canvas = renderEyeSymmetry(image, plan, { height, width }, createCanvas);
  const blob = await canvasToPngBlob(canvas);
  if (signal?.aborted || blob.size > maxCorrectedBytes) return null;

  return { blob, height, plan, width };
}

/** Samples the centre of each eye and compares their iris colours. */
function eyesHaveDifferentColours(
  image: CanvasImageSource,
  landmarks: ManualLandmarks,
  size: { height: number; width: number },
  createCanvas: EyeSymmetryCanvasFactory,
): boolean {
  const eyeDistance = Math.abs(landmarks.rightEye.x - landmarks.leftEye.x);
  const sample = (center: LandmarkPoint, side: "left" | "right") => {
    const region = landmarks.eyeRegions?.[side];
    const halfWidth = (region ? region.radiusX : eyeDistance * 0.25) * irisSampleShare * size.width;
    const halfHeight = (region ? region.radiusY : eyeDistance * 0.25) * irisSampleShare * size.height;
    const canvas = createCanvas(irisSampleSize, irisSampleSize);
    const context = requireContext(canvas);
    context.drawImage(
      image,
      center.x * size.width - halfWidth,
      center.y * size.height - halfHeight,
      Math.max(1, halfWidth * 2),
      Math.max(1, halfHeight * 2),
      0,
      0,
      irisSampleSize,
      irisSampleSize,
    );
    return irisColour(context.getImageData(0, 0, irisSampleSize, irisSampleSize).data);
  };
  return irisColoursDiffer(sample(landmarks.leftEye, "left"), sample(landmarks.rightEye, "right"));
}

function loadImageElement(url: string, signal?: AbortSignal): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    const abort = () => {
      image.onload = null;
      image.onerror = null;
      reject(new DOMException("Eye symmetry correction was cancelled", "AbortError"));
    };
    if (signal?.aborted) {
      abort();
      return;
    }
    signal?.addEventListener("abort", abort, { once: true });
    image.onload = () => {
      signal?.removeEventListener("abort", abort);
      resolve(image);
    };
    image.onerror = () => {
      signal?.removeEventListener("abort", abort);
      reject(new Error("Unable to load the generated image for the eye symmetry correction"));
    };
    // The API is another origin than the screen; it answers with CORS, so the canvas stays readable.
    image.crossOrigin = "anonymous";
    image.src = url;
  });
}

function canvasToPngBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) {
        resolve(blob);
        return;
      }
      reject(new Error("Unable to encode the corrected image"));
    }, "image/png");
  });
}
