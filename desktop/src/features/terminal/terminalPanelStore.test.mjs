import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  resetTerminalPanel,
  setTerminalPanelMode,
  setTerminalSessionChannels,
  toggleTerminalPanel,
  getTerminalPanelSnapshotForTests,
} from "./terminalPanelStore.ts";

beforeEach(resetTerminalPanel);

test("panel toggles between closed and the docked default", () => {
  toggleTerminalPanel();
  assert.equal(getTerminalPanelSnapshotForTests().mode, "docked");
  toggleTerminalPanel();
  assert.equal(getTerminalPanelSnapshotForTests().mode, "closed");
  setTerminalPanelMode("maximized");
  toggleTerminalPanel();
  assert.equal(getTerminalPanelSnapshotForTests().mode, "closed");
});

test("session channel identities are de-duplicated", () => {
  setTerminalSessionChannels(["one", "one", "two"]);
  // Regression guard: accepting an iterable (rather than Session objects) keeps
  // this store UI-only and prevents mutable PTYs from leaking into header state.
  setTerminalSessionChannels(new Set(["one", "two"]));
  assert.deepEqual(
    [...getTerminalPanelSnapshotForTests().sessionChannelIds],
    ["one", "two"],
  );
});

test("reset returns a clean panel snapshot", () => {
  setTerminalPanelMode("maximized");
  setTerminalSessionChannels(["old-community"]);
  const previous = getTerminalPanelSnapshotForTests();

  resetTerminalPanel();

  const next = getTerminalPanelSnapshotForTests();
  assert.notEqual(next, previous);
  assert.equal(next.mode, "closed");
  assert.deepEqual([...next.sessionChannelIds], []);
});
