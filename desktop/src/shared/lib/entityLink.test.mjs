import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  buildCommitLink,
  buildIssueLink,
  buildProjectLink,
  buildPullRequestLink,
  buildRepoLink,
  entityLinkProjectRouteId,
  ENTITY_LINK_TABS,
  isEntityLink,
  isLinkableCoordinate,
  parseEntityLink,
} from "./entityLink.ts";

const GOLDEN = JSON.parse(
  readFileSync(
    new URL("../../../../test-fixtures/entity-links.json", import.meta.url),
    "utf8",
  ),
);
const OWNER = GOLDEN.owner;
const EVENT_ID = GOLDEN.eventId;

// This fixture is also consumed by nimino-cli and the Tauri deep-link validator.
test("builders emit the canonical cross-language link format", () => {
  assert.equal(
    buildPullRequestLink({ id: EVENT_ID, owner: OWNER, dtag: GOLDEN.dtag }),
    GOLDEN.links.pullRequest,
  );
  assert.equal(
    buildIssueLink({ id: EVENT_ID, owner: OWNER, dtag: GOLDEN.dtag }),
    GOLDEN.links.issue,
  );
  assert.equal(
    buildRepoLink({ owner: OWNER, dtag: GOLDEN.dtag }),
    GOLDEN.links.repository,
  );
  assert.equal(
    buildProjectLink({ owner: OWNER, dtag: GOLDEN.dtag }),
    GOLDEN.links.project,
  );
  assert.equal(
    buildCommitLink({
      commitHash: EVENT_ID,
      owner: OWNER,
      dtag: GOLDEN.dtag,
    }),
    GOLDEN.links.commit,
  );
  assert.deepEqual(ENTITY_LINK_TABS, GOLDEN.tabs);
  for (const dtag of GOLDEN.validDtags) {
    assert.equal(isLinkableCoordinate(OWNER, dtag), true);
  }
  for (const dtag of GOLDEN.invalidDtags) {
    assert.equal(isLinkableCoordinate(OWNER, dtag), false);
  }
});

test("builders reject invalid identifiers", () => {
  assert.throws(() =>
    buildRepoLink({ owner: "not-a-pubkey", dtag: "nimino-world" }),
  );
  assert.throws(() => buildRepoLink({ owner: OWNER, dtag: ".hidden" }));
  assert.throws(() => buildRepoLink({ owner: OWNER, dtag: "a..b" }));
  assert.throws(() =>
    buildPullRequestLink({ id: "short", owner: OWNER, dtag: "nimino-world" }),
  );
});

test("parseEntityLink round-trips built links", () => {
  const link = buildPullRequestLink({
    id: EVENT_ID,
    owner: OWNER,
    dtag: "nimino-world",
  });
  assert.deepEqual(parseEntityLink(link), {
    ok: true,
    value: { type: "pr", id: EVENT_ID, owner: OWNER, dtag: "nimino-world" },
  });

  const repoLink = buildRepoLink({ owner: OWNER, dtag: "nimino-world" });
  assert.deepEqual(parseEntityLink(repoLink), {
    ok: true,
    value: { type: "repo", owner: OWNER, dtag: "nimino-world" },
  });

  const projectLink = buildProjectLink({ owner: OWNER, dtag: "nimino-world" });
  assert.deepEqual(parseEntityLink(projectLink), {
    ok: true,
    value: { type: "project", owner: OWNER, dtag: "nimino-world" },
  });
});

test("commit links select an exact repository commit", () => {
  const link = buildCommitLink({
    commitHash: EVENT_ID,
    owner: OWNER,
    dtag: "nimino-world",
  });
  assert.equal(
    link,
    `nimino://repo?owner=${OWNER}&d=nimino-world&tab=commits&commit=${EVENT_ID}`,
  );
  assert.deepEqual(parseEntityLink(link), {
    ok: true,
    value: {
      type: "repo",
      owner: OWNER,
      dtag: "nimino-world",
      tab: "commits",
      commitHash: EVENT_ID,
    },
  });
  assert.deepEqual(
    parseEntityLink(
      `nimino://repo?owner=${OWNER}&d=nimino-world&tab=files&commit=${EVENT_ID}`,
    ),
    { ok: false, reason: "invalid-commit" },
  );
});

test("parseEntityLink lowercase-normalizes hex identifiers", () => {
  const parsed = parseEntityLink(
    `nimino://issue?id=${EVENT_ID.toUpperCase()}&owner=${OWNER.toUpperCase()}&d=nimino-world`,
  );
  assert.deepEqual(parsed, {
    ok: true,
    value: { type: "issue", id: EVENT_ID, owner: OWNER, dtag: "nimino-world" },
  });
});

test("parseEntityLink rejects malformed links", () => {
  const cases = [
    ["not a url at all", "invalid-url"],
    [`https://pr?id=${EVENT_ID}&owner=${OWNER}&d=repo`, "wrong-scheme"],
    [`nimino://message?channel=x&id=${EVENT_ID}`, "wrong-host"],
    [`nimino://pr?id=${EVENT_ID}&owner=nope&d=repo`, "invalid-owner"],
    [`nimino://pr?id=${EVENT_ID}&owner=${OWNER}&d=.hidden`, "invalid-dtag"],
    [`nimino://pr?id=${EVENT_ID}&owner=${OWNER}`, "invalid-dtag"],
    [`nimino://pr?owner=${OWNER}&d=repo`, "invalid-id"],
    [`nimino://issue?id=short&owner=${OWNER}&d=repo`, "invalid-id"],
  ];
  for (const [href, reason] of cases) {
    assert.deepEqual(parseEntityLink(href), { ok: false, reason }, href);
  }
});

test("isEntityLink matches entity hosts and excludes message links", () => {
  assert.equal(isEntityLink(`nimino://pr?id=${EVENT_ID}`), true);
  assert.equal(isEntityLink(`nimino://issue?id=${EVENT_ID}`), true);
  assert.equal(isEntityLink(`nimino://repo?owner=${OWNER}`), true);
  assert.equal(isEntityLink(`nimino://project?owner=${OWNER}`), true);
  assert.equal(isEntityLink("nimino://message?channel=x&id=y"), false);
  assert.equal(isEntityLink("https://github.com/asopitech-labs/nimino"), false);
  assert.equal(isEntityLink(null), false);
});

test("entityLinkProjectRouteId emits the canonical 30617 coordinate route id", () => {
  const parsed = parseEntityLink(
    buildRepoLink({ owner: OWNER, dtag: "nimino-world" }),
  );
  assert.ok(parsed.ok);
  assert.equal(
    entityLinkProjectRouteId(parsed.value),
    `30617:${OWNER}:nimino-world`,
  );
});

test("entityLinkProjectRouteId routes project links to the 30621 coordinate", () => {
  const parsed = parseEntityLink(
    buildProjectLink({ owner: OWNER, dtag: "nimino-world" }),
  );
  assert.ok(parsed.ok);
  assert.equal(
    entityLinkProjectRouteId(parsed.value),
    `30621:${OWNER}:nimino-world`,
  );
});

test("coordinate links carry an optional workspace tab", () => {
  const link = buildProjectLink({
    owner: OWNER,
    dtag: "nimino-world",
    tab: "prs",
  });
  assert.equal(link, `nimino://project?owner=${OWNER}&d=nimino-world&tab=prs`);
  assert.deepEqual(parseEntityLink(link), {
    ok: true,
    value: { type: "project", owner: OWNER, dtag: "nimino-world", tab: "prs" },
  });

  const repoLink = buildRepoLink({
    owner: OWNER,
    dtag: "nimino-world",
    tab: "issues",
  });
  assert.deepEqual(parseEntityLink(repoLink), {
    ok: true,
    value: { type: "repo", owner: OWNER, dtag: "nimino-world", tab: "issues" },
  });

  // The default overview has no tab spelling; unknown values are rejected
  // rather than silently dropped, and event links accept no tab at all.
  assert.throws(() =>
    buildRepoLink({ owner: OWNER, dtag: "nimino-world", tab: "overview" }),
  );
  assert.deepEqual(
    parseEntityLink(`nimino://repo?owner=${OWNER}&d=nimino-world&tab=overview`),
    { ok: false, reason: "invalid-tab" },
  );
  assert.deepEqual(
    parseEntityLink(`nimino://repo?owner=${OWNER}&d=nimino-world&tab=`),
    { ok: false, reason: "invalid-tab" },
  );
  assert.deepEqual(
    parseEntityLink(
      `nimino://pr?id=${EVENT_ID}&owner=${OWNER}&d=nimino-world&tab=prs`,
    ),
    { ok: false, reason: "unknown-param" },
  );
});

test("isLinkableCoordinate gates coordinates the link format cannot express", () => {
  assert.equal(isLinkableCoordinate(OWNER, "nimino-world"), true);
  assert.equal(isLinkableCoordinate(OWNER, "a".repeat(64)), true);
  // Addressable d-tags allow far more than the link charset does.
  assert.equal(isLinkableCoordinate(OWNER, "a".repeat(65)), false);
  assert.equal(isLinkableCoordinate(OWNER, "has space"), false);
  assert.equal(isLinkableCoordinate(OWNER, ".hidden"), false);
  assert.equal(isLinkableCoordinate("not-a-pubkey", "nimino-world"), false);
});

test("parseEntityLink rejects noncanonical extras", () => {
  // Unexpected path segments — reserved for future versioning.
  assert.deepEqual(
    parseEntityLink(
      `nimino://pr/ignored?id=${EVENT_ID}&owner=${OWNER}&d=nimino-world`,
    ),
    { ok: false, reason: "unexpected-path" },
  );
  // Fragment — not part of the canonical format.
  assert.deepEqual(
    parseEntityLink(`nimino://repo?owner=${OWNER}&d=nimino-world#section`),
    { ok: false, reason: "unexpected-fragment" },
  );
  // Unknown query parameter — reject to preserve forward-compat posture.
  assert.deepEqual(
    parseEntityLink(
      `nimino://repo?owner=${OWNER}&d=nimino-world&relay=wss%3A%2F%2Frelay.example`,
    ),
    { ok: false, reason: "unknown-param" },
  );
  // Duplicate required parameter — reject.
  assert.deepEqual(
    parseEntityLink(
      `nimino://repo?owner=${OWNER}&d=nimino-world&owner=${OWNER}`,
    ),
    { ok: false, reason: "duplicate-param" },
  );
});
