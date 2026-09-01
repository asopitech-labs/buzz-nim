import assert from "node:assert/strict";
import { test } from "node:test";

import { projectExternalRefUrl } from "./projectExternalUrl.ts";

test("opens the selected GitHub branch", () => {
  assert.equal(
    projectExternalRefUrl(
      "https://github.com/asopitech-labs/nimino",
      "fix/agent-profile-about-preserve",
    ),
    "https://github.com/asopitech-labs/nimino/tree/fix%2Fagent-profile-about-preserve",
  );
});

test("normalizes clone URLs before adding the selected ref", () => {
  assert.equal(
    projectExternalRefUrl(
      "https://github.com/asopitech-labs/nimino.git/",
      "main",
    ),
    "https://github.com/asopitech-labs/nimino/tree/main",
  );
});

test("keeps unsupported and unscoped URLs unchanged", () => {
  assert.equal(
    projectExternalRefUrl("https://gitlab.com/asopitech-labs/nimino", "main"),
    "https://gitlab.com/asopitech-labs/nimino",
  );
  assert.equal(
    projectExternalRefUrl("https://github.com/asopitech-labs/nimino", null),
    "https://github.com/asopitech-labs/nimino",
  );
  assert.equal(projectExternalRefUrl("not a URL", "main"), "not a URL");
});
