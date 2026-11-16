import { describe, expect, it } from "vitest";
import {
  analyzeEyeSymmetry,
  drawEyeSymmetry,
  type EyeSymmetryPlan,
  eyeSymmetryThresholds,
  irisColour,
  irisColoursDiffer,
  isOneEyeClosed,
} from "./eyeSymmetry";
import { createDefaultLandmarks, type EyeRegion, type ManualLandmarks } from "./landmarks";

function symmetricLandmarks(overrides: Partial<ManualLandmarks> = {}): ManualLandmarks {
  return { ...createDefaultLandmarks(1, 1), ...overrides };
}

function eyeRegion(radiusX: number, radiusY = 0.05): EyeRegion {
  return { radiusBottomY: radiusY, radiusTopY: radiusY, radiusX, radiusY };
}

type RecordedCall = { args: unknown[]; type: string };

function createFakeContext() {
  const calls: RecordedCall[] = [];
  const record =
    (type: string) =>
    (...args: unknown[]) => {
      calls.push({ args, type });
    };
  return {
    calls,
    arc: record("arc"),
    beginPath: record("beginPath"),
    clearRect: record("clearRect"),
    createRadialGradient: () => ({ addColorStop: () => undefined }),
    drawImage: record("drawImage"),
    fill: record("fill"),
    fillStyle: "",
    globalCompositeOperation: "source-over",
    restore: record("restore"),
    save: record("save"),
    scale: record("scale"),
    translate: record("translate"),
  };
}

/**
 * The offscreen patch and mask canvases log into their own context, so the assertions below see only the calls that
 * land on the image being corrected.
 */
function createRenderTarget() {
  const context = createFakeContext();
  const offscreen = createFakeContext();
  const createCanvas = (width: number, height: number) =>
    ({ getContext: () => offscreen, height, width }) as unknown as HTMLCanvasElement;
  return { context, createCanvas, offscreen };
}

/** RGBA pixels: `coloured` of them in one colour, the rest white. */
function pixels(total: number, coloured: number, rgb: [number, number, number]) {
  const data = new Uint8ClampedArray(total * 4);
  for (let index = 0; index < total; index += 1) {
    const [red, green, blue] = index < coloured ? rgb : [255, 255, 255];
    data.set([red, green, blue, 255], index * 4);
  }
  return data;
}

describe("eye symmetry plan", () => {
  it("leaves an already symmetric eye pair alone", () => {
    expect(analyzeEyeSymmetry(symmetricLandmarks())).toBeNull();
  });

  it("ignores a difference inside the detection tolerance", () => {
    const withinTolerance = eyeSymmetryThresholds.heightDelta * 0.5;
    const landmarks = symmetricLandmarks({
      leftEye: { x: 0.42, y: 0.42 },
      rightEye: { x: 0.58, y: 0.42 + withinTolerance },
    });

    expect(analyzeEyeSymmetry(landmarks)).toBeNull();
  });

  it("levels the pair on their shared mean height, mirrored on the face centreline", () => {
    const plan = analyzeEyeSymmetry(
      symmetricLandmarks({ leftEye: { x: 0.42, y: 0.4 }, rightEye: { x: 0.58, y: 0.46 } }),
    );

    expect(plan).not.toBeNull();
    expect(plan?.eyeY).toBeCloseTo(0.43, 4);
    expect(plan?.axisX).toBeCloseTo(0.5, 4);
    expect(plan?.halfSpacing).toBeCloseTo(0.08, 4);
    expect(plan?.heightDelta).toBeCloseTo(0.06, 4);
    // No detected eye region: the patch falls back to a share of the eye distance and never crosses the centreline.
    expect(plan?.patchHalfWidth).toBeLessThanOrEqual(0.08 * eyeSymmetryThresholds.patchShareOfSpacing);
    expect(plan?.master).toBe("left");
    expect(plan?.masterCenter).toEqual({ x: 0.42, y: 0.4 });
  });

  it("makes the patch tall enough to cover a large difference in height", () => {
    const plan = analyzeEyeSymmetry(
      symmetricLandmarks({ leftEye: { x: 0.42, y: 0.36 }, rightEye: { x: 0.58, y: 0.5 } }),
    );

    expect(plan?.heightDelta).toBeCloseTo(0.14, 4);
    expect(plan?.patchHalfHeight).toBeGreaterThanOrEqual(0.07);
  });

  it("mirrors the eye the detector trusts most", () => {
    const landmarks = symmetricLandmarks({ leftEye: { x: 0.42, y: 0.4 }, rightEye: { x: 0.58, y: 0.46 } });

    expect(analyzeEyeSymmetry(landmarks, { left: 0.75, right: 0.3 })?.master).toBe("left");
    expect(analyzeEyeSymmetry(landmarks, { left: 0.3, right: 0.75 })?.master).toBe("right");
  });

  it("mirrors the larger eye when the confidence says nothing", () => {
    const plan = analyzeEyeSymmetry(
      symmetricLandmarks({
        leftEye: { x: 0.42, y: 0.42 },
        rightEye: { x: 0.58, y: 0.42 },
        eyeRegions: { left: eyeRegion(0.03, 0.03), right: eyeRegion(0.08, 0.08) },
      }),
      { left: 0.5, right: 0.52 },
    );

    expect(plan?.master).toBe("right");
    // The larger master region sets the patch size, still capped before the centreline.
    expect(plan?.patchHalfWidth).toBeCloseTo(0.08 * eyeSymmetryThresholds.patchShareOfSpacing, 4);
  });

  it("corrects a pair that is not centred on the face midline", () => {
    const plan = analyzeEyeSymmetry(
      symmetricLandmarks({ leftEye: { x: 0.36, y: 0.42 }, rightEye: { x: 0.6, y: 0.42 } }),
    );

    expect(plan?.spacingDelta).toBeCloseTo(1 / 6, 3);
    expect(plan?.axisX).toBeCloseTo(0.5, 4);
  });

  it("falls back to the eye midpoint when the landmarks disagree about the face axis", () => {
    const landmarks = symmetricLandmarks({
      chin: { x: 0.9, y: 0.7 },
      jawLeft: { x: 0.9, y: 0.6 },
      jawRight: { x: 0.92, y: 0.6 },
      mouthCenter: { x: 0.9, y: 0.57 },
      leftEye: { x: 0.42, y: 0.4 },
      rightEye: { x: 0.58, y: 0.46 },
    });

    expect(analyzeEyeSymmetry(landmarks)?.axisX).toBeCloseTo(0.5, 4);
  });

  it("gives up on landmarks too close together to hold two eyes", () => {
    const landmarks = symmetricLandmarks({ leftEye: { x: 0.5, y: 0.4 }, rightEye: { x: 0.51, y: 0.46 } });

    expect(analyzeEyeSymmetry(landmarks)).toBeNull();
  });

  it("keeps a wink: one closed eye is meant, not a mistake", () => {
    const landmarks = symmetricLandmarks({
      leftEye: { x: 0.42, y: 0.42 },
      rightEye: { x: 0.58, y: 0.43 },
      // The right eye is a flat closed line, the left one open.
      eyeRegions: { left: eyeRegion(0.05, 0.05), right: eyeRegion(0.05, 0.01) },
    });

    expect(isOneEyeClosed(landmarks)).toBe(true);
    expect(analyzeEyeSymmetry(landmarks)).toBeNull();
  });

  it("still corrects two open eyes of different sizes", () => {
    const landmarks = symmetricLandmarks({
      leftEye: { x: 0.42, y: 0.42 },
      rightEye: { x: 0.58, y: 0.42 },
      eyeRegions: { left: eyeRegion(0.05, 0.05), right: eyeRegion(0.04, 0.035) },
    });

    expect(isOneEyeClosed(landmarks)).toBe(false);
    expect(analyzeEyeSymmetry(landmarks)?.master).toBe("left");
  });
});

describe("iris colours", () => {
  it("reads the hue of the coloured pixels and ignores white and dark ones", () => {
    const blue = irisColour(pixels(100, 40, [40, 90, 220]));

    expect(blue.colouredShare).toBeCloseTo(0.4, 4);
    expect(blue.hue).toBeGreaterThan(200);
    expect(blue.hue).toBeLessThan(240);
    expect(irisColour(pixels(100, 40, [20, 20, 20])).colouredShare).toBe(0);
  });

  it("tells two different eye colours apart", () => {
    const blue = irisColour(pixels(100, 40, [40, 90, 220]));
    const gold = irisColour(pixels(100, 40, [230, 170, 30]));

    expect(irisColoursDiffer(blue, gold)).toBe(true);
  });

  it("treats close shades of one colour as the same", () => {
    const blue = irisColour(pixels(100, 40, [40, 90, 220]));
    const deeperBlue = irisColour(pixels(100, 30, [30, 60, 200]));

    expect(irisColoursDiffer(blue, deeperBlue)).toBe(false);
  });

  it("wraps around the red end of the hue circle", () => {
    const red = irisColour(pixels(100, 40, [220, 30, 40]));
    const crimson = irisColour(pixels(100, 40, [220, 40, 30]));

    expect(irisColoursDiffer(red, crimson)).toBe(false);
  });

  it("tells a coloured iris from a grey or black one, but not two dark ones", () => {
    const blue = irisColour(pixels(100, 40, [40, 90, 220]));
    const black = irisColour(pixels(100, 40, [15, 15, 15]));

    expect(irisColoursDiffer(blue, black)).toBe(true);
    expect(irisColoursDiffer(black, irisColour(pixels(100, 40, [30, 30, 30])))).toBe(false);
  });
});

describe("eye symmetry rendering", () => {
  const plan: EyeSymmetryPlan = {
    axisX: 0.5,
    eyeY: 0.43,
    halfSpacing: 0.08,
    patchHalfWidth: 0.07,
    patchHalfHeight: 0.06,
    master: "left",
    masterCenter: { x: 0.4, y: 0.4 },
    heightDelta: 0.06,
    spacingDelta: 0.02,
    sizeRatio: 0,
  };
  const source = { height: 1100, width: 800 } as unknown as CanvasImageSource;

  it("paints the base image and one mirrored copy of the master eye", () => {
    const { context, createCanvas, offscreen } = createRenderTarget();
    drawEyeSymmetry(
      context as unknown as CanvasRenderingContext2D,
      source,
      plan,
      { height: 1100, width: 800 },
      createCanvas,
    );

    expect(context.calls[0]).toEqual({ args: [0, 0, 800, 1100], type: "clearRect" });
    expect(context.calls[1]?.type).toBe("drawImage");
    expect(context.calls[1]?.args[0]).toBe(source);
    expect(context.calls.filter((call) => call.type === "translate").map((call) => call.args)).toEqual([
      [336, 473],
      [464, 473],
    ]);
    // Only the eye opposite the master is mirrored.
    expect(context.calls.filter((call) => call.type === "scale").map((call) => call.args)).toEqual([[-1, 1]]);
    // Base image plus the two pasted patches.
    expect(context.calls.filter((call) => call.type === "drawImage")).toHaveLength(3);
    // The patch is cropped around the master eye, so both corrected eyes keep that eye's own pixels.
    expect(offscreen.calls[0]).toEqual({
      args: [source, 264, 374, 112, 132, 0, 0, 112, 132],
      type: "drawImage",
    });
  });

  it("keeps the master's own side unmirrored when the right eye is the master", () => {
    const { context, createCanvas } = createRenderTarget();
    drawEyeSymmetry(
      context as unknown as CanvasRenderingContext2D,
      source,
      { ...plan, master: "right" },
      { height: 1100, width: 800 },
      createCanvas,
    );

    expect(context.calls.filter((call) => call.type === "scale").map((call) => call.args)).toEqual([[-1, 1]]);
    expect(context.calls.filter((call) => call.type === "translate").map((call) => call.args)).toEqual([
      [336, 473],
      [464, 473],
    ]);
  });
});
