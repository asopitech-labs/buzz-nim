import assert from "node:assert/strict";
import { test } from "node:test";
import { QueryClient } from "@tanstack/react-query";

import { pageOlderMessagesUntilRowFloor } from "./pageOlderMessages.ts";

test("deduplicates paging per query client, not across communities", async () => {
  const firstClient = new QueryClient();
  const secondClient = new QueryClient();

  const first = pageOlderMessagesUntilRowFloor(
    firstClient,
    "shared-channel-id",
    () => false,
  );
  const duplicate = pageOlderMessagesUntilRowFloor(
    firstClient,
    "shared-channel-id",
    () => false,
  );
  const otherCommunity = pageOlderMessagesUntilRowFloor(
    secondClient,
    "shared-channel-id",
    () => false,
  );

  assert.equal(first, duplicate);
  assert.notEqual(first, otherCommunity);
  await Promise.all([first, otherCommunity]);
});
