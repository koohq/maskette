import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { detectAnimeLandmarks } from "../editor/deformation/animeLandmarkDetector";
import { createDefaultLandmarks, type ManualLandmarks } from "../editor/deformation/landmarks";
import { correctEyeSymmetry, supportsEyeSymmetryCorrection } from "./eyeSymmetryCorrection";

vi.mock("../editor/deformation/animeLandmarkDetector", () => ({
  detectAnimeLandmarks: vi.fn(),
}));

const imageWidth = 800;
const imageHeight = 1100;

class FakeImage {
  crossOrigin: string | null = null;
  height = imageHeight;
  naturalHeight = imageHeight;
  naturalWidth = imageWidth;
  onerror: (() => void) | null = null;
  onload: (() => void) | null = null;
  width = imageWidth;

  set src(_value: string) {
    queueMicrotask(() => this.onload?.());
  }
}

const unevenEyes = { leftEye: { x: 0.42, y: 0.4 }, rightEye: { x: 0.58, y: 0.46 } };

function landmarks(overrides: Partial<ManualLandmarks> = {}): ManualLandmarks {
  return { ...createDefaultLandmarks(1, 1), ...overrides };
}

/** The colour each iris sample reads, in the order they are taken: left eye, then right. */
let irisSamples: Array<[number, number, number]> = [];

function samplePixels() {
  const rgb = irisSamples.shift() ?? [40, 90, 220];
  const data = new Uint8ClampedArray(32 * 32 * 4);
  for (let index = 0; index < 32 * 32; index += 1) data.set([...rgb, 255], index * 4);
  return { data };
}

function fakeContext() {
  return {
    arc: () => undefined,
    beginPath: () => undefined,
    clearRect: () => undefined,
    createRadialGradient: () => ({ addColorStop: () => undefined }),
    drawImage: () => undefined,
    fill: () => undefined,
    fillStyle: "",
    getImageData: samplePixels,
    globalCompositeOperation: "source-over",
    restore: () => undefined,
    save: () => undefined,
    scale: () => undefined,
    translate: () => undefined,
  };
}

function detect(controls: ManualLandmarks | null) {
  vi.mocked(detectAnimeLandmarks).mockResolvedValue(
    (controls ? { controls, debug: { points: [] }, details: [] } : null) as never,
  );
}

const originalGetContext = HTMLCanvasElement.prototype.getContext;
const originalToBlob = HTMLCanvasElement.prototype.toBlob;

beforeEach(() => {
  irisSamples = [];
  vi.stubGlobal("Image", FakeImage);
  Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
    configurable: true,
    value: () => fakeContext(),
  });
  Object.defineProperty(HTMLCanvasElement.prototype, "toBlob", {
    configurable: true,
    value: (callback: BlobCallback) => callback(new Blob(["corrected"], { type: "image/png" })),
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  Object.defineProperty(HTMLCanvasElement.prototype, "getContext", { configurable: true, value: originalGetContext });
  Object.defineProperty(HTMLCanvasElement.prototype, "toBlob", { configurable: true, value: originalToBlob });
});

describe("eye symmetry correction roles", () => {
  it("covers the generated stage roles only", () => {
    expect(supportsEyeSymmetryCorrection("design")).toBe(true);
    expect(supportsEyeSymmetryCorrection("design_turnaround")).toBe(true);
    expect(supportsEyeSymmetryCorrection("front")).toBe(true);
    expect(supportsEyeSymmetryCorrection("turnaround")).toBe(true);
    expect(supportsEyeSymmetryCorrection("reference")).toBe(false);
    expect(supportsEyeSymmetryCorrection(undefined)).toBe(false);
  });

  it("never rewrites a role it does not own", async () => {
    detect(landmarks(unevenEyes));

    await expect(correctEyeSymmetry({ imageUrl: "/source.png", role: "reference" })).resolves.toBeNull();
    expect(detectAnimeLandmarks).not.toHaveBeenCalled();
  });
});

describe("eye symmetry correction", () => {
  it("returns corrected pixels for an uneven pair", async () => {
    detect(landmarks(unevenEyes));

    const correction = await correctEyeSymmetry({ imageUrl: "/source.png", role: "front" });

    expect(correction?.plan.eyeY).toBeCloseTo(0.43, 4);
    expect(correction?.plan.master).toBe("left");
    expect(correction?.width).toBe(imageWidth);
    expect(correction?.height).toBe(imageHeight);
    await expect(correction?.blob.text()).resolves.toBe("corrected");
    // Strict single-face detection, so a weak fit is rejected instead of guessed.
    expect(detectAnimeLandmarks).toHaveBeenCalledWith(expect.anything(), undefined, true, undefined);
    // The API is another origin: the image is loaded with CORS so its pixels can be read.
    const [loaded] = vi.mocked(detectAnimeLandmarks).mock.calls[0] ?? [];
    expect((loaded as unknown as FakeImage | undefined)?.crossOrigin).toBe("anonymous");
  });

  it("pins the detection to the front panel of a four-view sheet", async () => {
    detect(landmarks(unevenEyes));

    await correctEyeSymmetry({ imageUrl: "/source.png", role: "turnaround" });

    expect(detectAnimeLandmarks).toHaveBeenCalledWith(expect.anything(), undefined, true, {
      height: imageHeight,
      width: imageWidth * 0.26,
      x: 0,
      y: 0,
    });
  });

  it("leaves two different eye colours alone", async () => {
    detect(landmarks(unevenEyes));
    irisSamples = [
      [40, 90, 220],
      [230, 170, 30],
    ];

    await expect(correctEyeSymmetry({ imageUrl: "/source.png", role: "front" })).resolves.toBeNull();
  });

  it("does nothing when the eye pair is already symmetric", async () => {
    detect(landmarks());

    await expect(correctEyeSymmetry({ imageUrl: "/source.png", role: "design" })).resolves.toBeNull();
  });

  it("does nothing when no face was found confidently", async () => {
    detect(null);

    await expect(correctEyeSymmetry({ imageUrl: "/source.png", role: "design" })).resolves.toBeNull();
  });

  it("honours an abort that arrives before the correction starts", async () => {
    detect(landmarks(unevenEyes));
    const controller = new AbortController();
    controller.abort();

    await expect(
      correctEyeSymmetry({ imageUrl: "/source.png", role: "front", signal: controller.signal }),
    ).rejects.toThrow();
  });
});
