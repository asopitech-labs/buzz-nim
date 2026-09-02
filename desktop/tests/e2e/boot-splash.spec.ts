import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

// Cold-boot splash hold: on a real boot the community resolves before the
// hidden Tauri window puts a frame on screen. E2E skips the hold by default;
// this spec opts back in via __NIMINO_E2E__.bootSplashHoldMs.

test.describe.configure({ timeout: 120_000 });

test("boot splash is stable with reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installMockBridge(page);
  // Registered after installMockBridge so it runs after the bridge's init
  // script and can extend the config it assigns.
  await page.addInitScript(() => {
    const testWindow = window as Window & {
      __NIMINO_E2E__?: { bootSplashHoldMs?: number };
    };
    testWindow.__NIMINO_E2E__ = {
      ...(testWindow.__NIMINO_E2E__ ?? {}),
      bootSplashHoldMs: 60_000,
    };
  });
  await page.goto("/");

  const gate = page.getByTestId("app-loading-gate");
  await expect(gate).toBeVisible();

  const runningAnimations = await gate.evaluate(
    (element) =>
      element
        .getAnimations({ subtree: true })
        .filter((animation) => animation.playState === "running").length,
  );
  expect(runningAnimations).toBe(0);
  expect(await gate.screenshot()).toEqual(await gate.screenshot());
});

test("boot splash overlay is skipped when the hold is zero (e2e default)", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await expect(page.locator("main")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("boot-splash-overlay")).toHaveCount(0);
});
