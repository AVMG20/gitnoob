import { describe, expect, it } from "vitest";
import { Text } from "@codemirror/state";
import {
  changeAt,
  changesOf,
  revertEdit,
  sameChange,
  stagePick,
  wordChanges,
} from "~/composables/changeCard";

const ctx = (n: number, content: string, old = n) => ({
  origin: " ",
  old_lineno: old,
  new_lineno: n,
  content,
});
const add = (n: number, content: string) => ({
  origin: "+",
  old_lineno: null,
  new_lineno: n,
  content,
});
const del = (n: number, content: string) => ({
  origin: "-",
  old_lineno: n,
  new_lineno: null,
  content,
});

// one, TWO (was two), three, four-new (added), five; "gone" removed after five.
const HUNKS = [
  {
    header: "@@ -1,6 +1,6 @@",
    lines: [
      ctx(1, "one"),
      del(2, "two"),
      add(2, "TWO"),
      ctx(3, "three"),
      add(4, "four-new"),
      ctx(5, "five", 4),
      del(5, "gone"),
      ctx(6, "six", 6),
    ],
  },
];
const NOW = "one\nTWO\nthree\nfour-new\nfive\nsix\n";

function apply(
  text: string,
  edit: { from: number; to: number; insert: string },
) {
  return text.slice(0, edit.from) + edit.insert + text.slice(edit.to);
}

describe("the changes behind the bars", () => {
  it("splits hunks into runs, as the bars do", () => {
    const all = changesOf(HUNKS);
    expect(all.map((one) => [one.kind, one.from, one.to])).toEqual([
      ["modified", 2, 2],
      ["added", 4, 4],
      ["removed", 6, 6],
    ]);
    expect(all[0]!.removed).toEqual(["two"]);
    expect(all[2]!.after).toBe(5);
    expect(changeAt(all, 4)?.kind).toBe("added");
    expect(changeAt(all, 1)).toBeNull();
  });

  it("puts each kind back with one edit", () => {
    const all = changesOf(HUNKS);
    const doc = Text.of(NOW.split("\n"));
    expect(apply(NOW, revertEdit(all[0]!, doc)!)).toBe(
      "one\ntwo\nthree\nfour-new\nfive\nsix\n",
    );
    expect(apply(NOW, revertEdit(all[1]!, doc)!)).toBe(
      "one\nTWO\nthree\nfive\nsix\n",
    );
    expect(apply(NOW, revertEdit(all[2]!, doc)!)).toBe(
      "one\nTWO\nthree\nfour-new\nfive\ngone\nsix\n",
    );
  });

  it("picks out the words that changed, whole", () => {
    const [change] = changesOf([
      {
        header: "@@ -1 +1 @@",
        lines: [
          del(1, "return total * rate;"),
          add(1, "return total * taxRate;"),
        ],
      },
    ]);
    const [ranges] = wordChanges(change!);
    expect(
      ranges!.map(([from, to]) => "return total * rate;".slice(from, to)),
    ).toEqual(["rate"]);
  });

  it("follows a change that moved down, and loses one that changed", () => {
    const [was] = changesOf(HUNKS);
    const moved = changesOf([
      {
        header: "@@ -1,2 +1,3 @@",
        lines: [
          add(1, "new top"),
          ctx(2, "one", 1),
          del(2, "two"),
          add(3, "TWO"),
        ],
      },
    ]);
    expect(sameChange(moved, was!)?.from).toBe(3);
    const other = changesOf([
      { header: "@@ -2 +2 @@", lines: [del(2, "two"), add(2, "Two")] },
    ]);
    expect(sameChange(other, was!)).toBeNull();
  });

  it("finds the same lines in the unstaged diff for staging, or says it is all staged", () => {
    const [change] = changesOf(HUNKS);
    // Against the index the file differs in two places; the second matches.
    const unstaged = [
      {
        header: "@@ -1,2 +1,2 @@",
        lines: [ctx(1, "one"), del(2, "two"), add(2, "TWO")],
      },
      { header: "@@ -9 +9 @@", lines: [del(9, "x"), add(9, "y")] },
    ];
    expect(stagePick(unstaged, change!)).toEqual({
      hunk: 0,
      lines: { added: [2], removed: [2] },
    });
    expect(stagePick([unstaged[1]!], change!)).toBeNull();
  });
});
