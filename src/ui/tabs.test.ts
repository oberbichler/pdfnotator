import { beforeEach, describe, expect, it } from "vitest";
import { state } from "../store";
import { doc, resetStore } from "../test-utils";
import { cycleTab, selectTab, tabActivity } from "./tabs";

describe("tabs", () => {
  beforeEach(() => resetStore(1, 2, 3));

  it("cycles through tabs and wraps around", () => {
    cycleTab(1);
    expect(state.activeId).toBe(2);
    cycleTab(-2);
    expect(state.activeId).toBe(3);
    cycleTab(1);
    expect(state.activeId).toBe(1);
  });

  it("selects by position, -1 = last", () => {
    selectTab(1);
    expect(state.activeId).toBe(2);
    selectTab(-1);
    expect(state.activeId).toBe(3);
    selectTab(7);
    expect(state.activeId).toBe(3);
  });

  it("shows what the tab's agent is doing", () => {
    expect(tabActivity(doc(1))).toBe("");
    doc(1).agentSeen = true;
    expect(tabActivity(doc(1))).toBe("busy");
    doc(1).listeners = 1;
    expect(tabActivity(doc(1))).toBe("on");
  });
});
