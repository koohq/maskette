/**
 * Time limit for tests that render the whole editor (`EditorWorkspace`). One render takes well under a second on an
 * idle machine, but a parallel build or a second test run can push it past Vitest's 5-second default, so these files
 * get a wider limit. Other tests keep the default, so a stuck test there still fails fast.
 */
export const EDITOR_TEST_TIMEOUT_MS = 60_000;
