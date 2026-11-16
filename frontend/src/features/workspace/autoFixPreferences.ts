import { readStored, writeStored } from "../../app/storage";

export const AUTO_EYE_SYMMETRY_STORAGE_KEY = "maskette.autoEyeSymmetry.v1";

/** Whether generated versions get their uneven eyes fixed. On unless the user turned it off in the settings. */
export function readAutoEyeSymmetry(): boolean {
  return readStored(AUTO_EYE_SYMMETRY_STORAGE_KEY) !== "0";
}

export function writeAutoEyeSymmetry(enabled: boolean): void {
  writeStored(AUTO_EYE_SYMMETRY_STORAGE_KEY, enabled ? "1" : "0");
}
