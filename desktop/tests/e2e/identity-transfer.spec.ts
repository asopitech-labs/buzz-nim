import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const RECOVERY_CODE =
  "nostrpair://8f4b8db31967ce14fef970a1ff1e8eecf19a430aa1c83875e2f5be68dcac0f1a?relay=wss%3A%2F%2Frelay.example.com&secret=87d5a8cfd5807a0cb44f728b67d88d6dcb8daf99be137c158f21a50c1e913c0a&v=1&mode=recover";

async function emitPairingEvent(page: Page, event: string, payload?: unknown) {
  await page.evaluate(
    async ({ eventName, eventPayload }) => {
      await window.__TAURI_INTERNALS__?.invoke?.("plugin:event|emit", {
        event: eventName,
        payload: eventPayload,
      });
    },
    { eventName: event, eventPayload: payload },
  );
}

async function openIdentityTransfer(page: Page) {
  await page.goto("/");
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-identity-transfer").click();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("authorized Desktop sends its identity to a fresh Desktop recovery session", async ({
  page,
}) => {
  await openIdentityTransfer(page);

  const section = page.getByTestId("settings-identity-transfer");
  await expect(
    section.getByRole("heading", { name: "Identity transfer" }),
  ).toBeVisible();
  await expect(
    section.getByText(
      "Send this signed-in Desktop identity only to a new Desktop you control.",
    ),
  ).toBeVisible();

  const input = section.getByTestId("identity-transfer-code");
  const connect = section.getByTestId("start-identity-transfer");
  await expect(connect).toBeDisabled();
  await input.fill(RECOVERY_CODE);
  await connect.click();
  await expect(
    section.getByText("Connecting to the receiving Desktop…"),
  ).toBeVisible();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__NIMINO_E2E_COMMAND_LOG__?.findLast(
            ({ command }) => command === "join_identity_recovery_pairing",
          )?.payload,
      ),
    )
    .toEqual({ pairingUri: RECOVERY_CODE });

  await emitPairingEvent(page, "pairing-sas-received", { sas: "123456" });
  await expect(section.getByTestId("identity-transfer-sas")).toHaveText(
    "123 456",
  );
  await expect(
    section.getByText(
      "Confirm only if you started recovery on a new Desktop you control.",
    ),
  ).toBeVisible();

  await section.getByTestId("confirm-identity-transfer").click();
  await expect(
    section.getByText("Sending identity to the receiving Desktop…"),
  ).toBeVisible();
  await emitPairingEvent(page, "pairing-complete");
  await expect(
    section.getByText("Identity transferred", { exact: true }),
  ).toBeVisible();
  await expect(
    section.getByText("The receiving Desktop can now sign with this identity."),
  ).toBeVisible();
});

test("cancel ignores late pairing events", async ({ page }) => {
  await openIdentityTransfer(page);
  const section = page.getByTestId("settings-identity-transfer");
  await section.getByTestId("identity-transfer-code").fill(RECOVERY_CODE);
  await section.getByTestId("start-identity-transfer").click();
  await emitPairingEvent(page, "pairing-sas-received", { sas: "123456" });
  await section.getByTestId("cancel-identity-transfer").click();

  await expect(
    section.getByText(
      "The codes didn't match. Identity transfer was canceled.",
    ),
  ).toBeVisible();
  await emitPairingEvent(page, "pairing-complete");
  await emitPairingEvent(page, "pairing-sas-received", { sas: "654321" });
  await expect(
    section.getByText("Identity transferred", { exact: true }),
  ).toHaveCount(0);
  await expect(section.getByTestId("identity-transfer-sas")).toHaveCount(0);
});
