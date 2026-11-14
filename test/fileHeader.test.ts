import { describe, expect, it } from "vitest";
import { headerState } from "~/composables/fileHeader";

const file = (extra = {}) => ({ path: "app/Cart.php", ...extra });
const none = {};

describe("the bar over an open file", () => {
  it("reads an unchanged tracked file: no badge, counts, git buttons or Diff", () => {
    const state = headerState(file(), none, {});
    expect(state.kind).toBe("unchanged");
    expect(state.badges).toEqual([]);
    expect(state.counts).toBe(false);
    expect(state.modes).toBe(false);
    expect(state.actions).toEqual([]);
  });

  it("offers Stage and Discard for a file changed and not staged", () => {
    const state = headerState(file(), { unstaged: { kind: "modified" } }, {});
    expect(state.kind).toBe("modified");
    expect(state.actions).toEqual(["discard", "stage"]);
    expect(state.counts).toBe(true);
    expect(state.modes).toBe(true);
  });

  it("offers only Unstage when every change is staged", () => {
    expect(
      headerState(file(), { staged: { kind: "modified" } }, {}).actions,
    ).toEqual(["unstage"]);
  });

  it("offers both when the file is partly staged", () => {
    const state = headerState(
      file(),
      { staged: { kind: "modified" }, unstaged: { kind: "modified" } },
      {},
    );
    expect(state.actions).toEqual(["discard", "stage", "unstage"]);
  });

  it("says a deleted or conflicted file is one", () => {
    expect(
      headerState(file(), { unstaged: { kind: "deleted" } }, {}).badges[0]!
        .text,
    ).toBe("deleted");
    const clash = headerState(
      file(),
      { unstaged: { kind: "modified" }, conflicted: true },
      {},
    );
    expect(clash.badges[0]!.text).toBe("conflicted");
  });

  it("calls an untracked file new, with Add and Delete, and a diff of all of it", () => {
    const state = headerState(file(), { unstaged: { kind: "untracked" } }, {});
    expect(state.kind).toBe("untracked");
    expect(state.badges).toEqual([]);
    expect(state.actions).toEqual(["add", "delete"]);
    expect(state.modes).toBe(true);
  });

  it("marks an ignored file and offers git nothing", () => {
    const state = headerState(file({ ignored: true }), none, {});
    expect(state.kind).toBe("ignored");
    expect(state.badges.map((one) => one.text)).toEqual(["ignored"]);
    expect(state.actions).toEqual([]);
    expect(state.modes).toBe(false);
  });

  it("says read-only with the reason", () => {
    const state = headerState(
      file({ readonly: "endings" }),
      none,
      {},
    );
    expect(state.badges[0]!.text).toBe("read-only");
    expect(state.badges[0]!.title).toMatch(/line endings/);
    const binary = headerState(
      file({ readonly: "binary" }),
      { unstaged: { kind: "modified" } },
      {},
    );
    expect(binary.badges.map((one) => one.text)).toEqual(["read-only"]);
    expect(binary.actions).toEqual(["discard", "stage"]);
  });

  it("shows a commit tab its hash and subject, and the way to the current file", () => {
    const state = headerState(
      file(),
      { unstaged: { kind: "modified" } },
      {
        commit: "a91c4e2000000000",
        subject: "Fix the cart total",
      },
    );
    expect(state.kind).toBe("commit");
    expect(state.commit).toEqual({
      short: "a91c4e2",
      subject: "Fix the cart total",
    });
    expect(state.actions).toEqual(["open-current"]);
  });

  it("leaves a file gone from disk and git to the missing-file page", () => {
    const state = headerState(file({ missing: true }), none, {});
    expect(state.kind).toBe("missing");
    expect(state.actions).toEqual([]);
    expect(state.counts).toBe(false);
  });
});
