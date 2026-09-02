import assert from "node:assert/strict";
import test from "node:test";

import {
  isWelcomeKickoffSettingUp,
  resolveWelcomeKickoffStagePhase,
} from "./useWelcomeKickoffStage.ts";

const base = {
  isWelcome: true,
  timelineSettled: true,
  hasMessages: false,
  timedOut: false,
};

test("stage stays hidden outside the Welcome channel", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("hidden", { ...base, isWelcome: false }),
    "hidden",
  );
  assert.equal(
    resolveWelcomeKickoffStagePhase("active", { ...base, isWelcome: false }),
    "hidden",
  );
});

test("stage waits for the timeline to settle before entering", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("hidden", {
      ...base,
      timelineSettled: false,
    }),
    "hidden",
  );
  assert.equal(resolveWelcomeKickoffStagePhase("hidden", base), "active");
});

test("stage never enters when messages already exist (revisit)", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("hidden", { ...base, hasMessages: true }),
    "hidden",
  );
});

test("first message resolves active setup immediately", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("active", { ...base, hasMessages: true }),
    "done",
  );
});

test("timeout resolves active setup immediately", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("active", { ...base, timedOut: true }),
    "done",
  );
});

test("only an active setup claims the team is being set up", () => {
  assert.equal(isWelcomeKickoffSettingUp("active"), true);
  for (const phase of ["hidden", "done"]) {
    assert.equal(
      isWelcomeKickoffSettingUp(phase),
      false,
      `${phase} must not claim setup is in progress`,
    );
  }
});

test("done is terminal and never replays on a still-empty timeline", () => {
  assert.equal(resolveWelcomeKickoffStagePhase("done", base), "done");
  assert.equal(
    resolveWelcomeKickoffStagePhase("done", { ...base, isWelcome: false }),
    "done",
  );
  assert.equal(
    resolveWelcomeKickoffStagePhase("done", { ...base, timedOut: true }),
    "done",
  );
});
