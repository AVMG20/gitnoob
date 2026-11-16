// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import SearchModal from "~/components/SearchModal.vue";
import AppModal from "~/components/AppModal.vue";
import { useGit } from "~/composables/useGit";
import { openTab, resetTabs, switchTabs, useTabs } from "~/composables/useTabs";
import {
  openSearch,
  resetSearch,
  search,
  takeDone,
  takeHits,
} from "~/composables/useSearch";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

const asked = vi.mocked(invoke);
const git = useGit();
const tabs = useTabs();
const isMac = navigator.userAgent.includes("Mac");
let wrapper: ReturnType<typeof mount> | null = null;
let found: { path: string; score: number; indices: number[] }[] = [];
let symbolsFound: {
  name: string;
  kind: string;
  path: string;
  line: number;
  column: number;
  indices: number[];
}[] = [];

const wait = (ms: number) => new Promise((done) => setTimeout(done, ms));

beforeEach(() => {
  resetTabs();
  resetSearch();
  switchTabs("/repos/shop");
  git.store.repo = { path: "/repos/shop", name: "shop", head: "main" } as never;
  git.store.status = {
    staged: [],
    unstaged: [{ path: "app/Cart.php", kind: "modified" }],
    conflicted: [],
  } as never;
  found = [];
  symbolsFound = [];
  asked.mockReset();
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === "find_files") return found;
    if (cmd === "find_symbols") return symbolsFound;
    if (cmd === "read_file") {
      return {
        text: "a\nb\nc\n",
        hash: "h",
        eol: "lf",
        bom: false,
        readonly: null,
        missing: false,
      };
    }
    return null;
  });
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  git.store.repo = null;
  git.store.status = null;
});

async function show(mode: "text" | "files" = "text") {
  openSearch(mode);
  wrapper = mount(SearchModal, {
    attachTo: document.body,
    global: { components: { AppModal } },
  });
  await flushPromises();
  return wrapper;
}

function press(
  w: ReturnType<typeof mount>,
  key: string,
  extra: KeyboardEventInit = {},
) {
  w.find('.search input[type="text"]').element.dispatchEvent(
    new KeyboardEvent("keydown", {
      key,
      bubbles: true,
      cancelable: true,
      ...extra,
    }),
  );
}

const searches = () =>
  asked.mock.calls.filter(([cmd]) => cmd === "search_text");

/** Answers the search that went out last, the way the backend's events would. */
function answer(
  hits: { path: string; lines: [number, string, [number, number][]][] }[],
  more = 0,
) {
  const request = search.request;
  for (const one of hits) {
    takeHits({
      request,
      path: one.path,
      hits: one.lines.map(([line, text, ranges]) => ({ line, text, ranges })),
    });
  }
  const matches = hits.reduce((sum, one) => sum + one.lines.length, 0);
  takeDone({
    request,
    matches,
    files: hits.length,
    more,
    searched: 10,
    cancelled: false,
    error: null,
  });
}

describe("searching inside files", () => {
  it("waits for typing to pause, then searches with the options as set", async () => {
    const w = await show();
    await w.find('[data-option="case"]').trigger("click");
    await w.find('[data-option="regex"]').trigger("click");
    await w.find('[data-option="changed"]').trigger("click");
    await w.find(".mask").setValue("*.php");
    await w.find('.search input[type="text"]').setValue("ta");
    await w.find('.search input[type="text"]').setValue("tax");
    await wait(50);
    expect(searches()).toHaveLength(0);
    await wait(150);
    await flushPromises();
    expect(searches()).toHaveLength(1);
    expect(searches()[0]![1]).toMatchObject({
      query: "tax",
      options: {
        case: true,
        word: false,
        regex: true,
        changed_only: true,
        glob: "*.php",
      },
    });
  });

  it("groups the hits by file, marks the matches, and counts what it left out", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("total");
    await wait(150);
    await flushPromises();
    answer(
      [
        {
          path: "app/Cart.php",
          lines: [
            [4, "$total = 0;", [[1, 6]]],
            [9, "return $total;", [[8, 13]]],
          ],
        },
        { path: "app/Order.php", lines: [[12, "total()", [[0, 5]]]] },
      ],
      40,
    );
    await flushPromises();
    const groups = w.findAll(".group");
    expect(groups.map((one) => one.find(".file").text())).toEqual([
      "Cart.php",
      "Order.php",
    ]);
    expect(groups[0]!.find(".letter").text()).toBe("M");
    expect(w.findAll(".item")).toHaveLength(3);
    expect(w.findAll(".item mark").map((one) => one.text())).toEqual([
      "total",
      "total",
      "total",
    ]);
    expect(w.text()).toContain("3 matches in 2 files");
    expect(w.text()).toContain("40 more not shown");
  });

  it("drops what an older search sends after a new one started", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("one");
    await wait(150);
    await flushPromises();
    const old = search.request;
    await w.find('.search input[type="text"]').setValue("two");
    await wait(150);
    await flushPromises();
    takeHits({
      request: old,
      path: "stale.php",
      hits: [{ line: 1, text: "one", ranges: [[0, 3]] }],
    });
    await flushPromises();
    expect(w.findAll(".item")).toHaveLength(0);
  });

  it("opens the hit under the cursor at its line on ↵, pinned and on the history, and closes", async () => {
    openTab({ path: "README.md", mode: "file" });
    const w = await show();
    await w.find('.search input[type="text"]').setValue("total");
    await wait(150);
    await flushPromises();
    answer([
      {
        path: "app/Cart.php",
        lines: [
          [4, "$total", [[1, 6]]],
          [9, "total", [[0, 5]]],
        ],
      },
    ]);
    await flushPromises();
    press(w, "ArrowDown");
    await flushPromises();
    press(w, "Enter");
    await flushPromises();
    const tab = tabs.state.tabs.find((one) => one.path === "app/Cart.php")!;
    expect(tabs.state.active).toBe(tab.id);
    expect(tab.preview).toBe(false);
    expect(tab.line).toBe(9);
    expect(tabs.state.landing).toMatchObject({ tab: tab.id, line: 9 });
    expect(tabs.state.back.at(-1)?.path).toBe("README.md");
    expect(search.open).toBe(false);
  });

  it("opens and stays open on ⌘↵, for walking through every hit", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("total");
    await wait(150);
    await flushPromises();
    answer([{ path: "app/Cart.php", lines: [[4, "$total", [[1, 6]]]] }]);
    await flushPromises();
    press(w, "Enter", { metaKey: isMac, ctrlKey: !isMac });
    expect(tabs.state.tabs.some((one) => one.path === "app/Cart.php")).toBe(
      true,
    );
    expect(search.open).toBe(true);
  });

  it("stops the search still running when the window closes or the box is cleared", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("total");
    await wait(150);
    await flushPromises();
    expect(search.running).toBe(true);
    const before = search.request;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    const last = searches().at(-1)![1] as { query: string; request: number };
    expect(last.query).toBe("");
    expect(last.request).toBeGreaterThan(before);

    openSearch("text");
    await w.find('.search input[type="text"]').setValue("");
    await wait(150);
    await flushPromises();
    expect((searches().at(-1)![1] as { query: string }).query).toBe("");
  });

  it("lets ↵ press an option button rather than open a result", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("total");
    await wait(150);
    await flushPromises();
    answer([{ path: "app/Cart.php", lines: [[4, "$total", [[1, 6]]]] }]);
    await flushPromises();
    w.find('[data-option="case"]').element.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        cancelable: true,
      }),
    );
    expect(tabs.state.tabs.some((one) => one.path === "app/Cart.php")).toBe(
      false,
    );
    expect(search.open).toBe(true);
  });

  it("closes on Esc", async () => {
    await show();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(search.open).toBe(false);
  });

  it("takes what was typed along when switching to file names", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("Cart");
    press(w, "N", {
      code: "KeyN",
      shiftKey: true,
      metaKey: isMac,
      ctrlKey: !isMac,
    });
    await flushPromises();
    expect(search.mode).toBe("files");
    expect(search.files.query).toBe("Cart");
    expect(
      (w.find('.search input[type="text"]').element as HTMLInputElement).value,
    ).toBe("Cart");
  });
});

describe("finding files by name", () => {
  it("lists recent files and then changed ones before anything is typed", async () => {
    openTab({ path: "app/Tax.php", mode: "file" });
    const w = await show("files");
    await wait(80);
    await flushPromises();
    expect(w.findAll(".group").map((one) => one.text())).toEqual([
      "Recent",
      "Changed",
    ]);
    expect(w.findAll(".item .file").map((one) => one.text())).toEqual([
      "Tax.php",
      "Cart.php",
    ]);
  });

  it("ranks recent, then changed, then the rest, whatever the score", async () => {
    openTab({ path: "app/Tax.php", mode: "file" });
    found = [
      { path: "lib/Taxi.php", score: 90, indices: [4, 5, 6] },
      { path: "app/Cart.php", score: 50, indices: [] },
      { path: "app/Tax.php", score: 10, indices: [4, 5, 6] },
    ];
    const w = await show("files");
    await w.find('.search input[type="text"]').setValue("ta");
    await wait(80);
    await flushPromises();
    const call = asked.mock.calls.find(([cmd]) => cmd === "find_files")!;
    expect(call[1]).toMatchObject({
      query: "ta",
      also: ["app/Tax.php", "app/Cart.php"],
    });
    expect(w.findAll(".item .file").map((one) => one.text())).toEqual([
      "Tax.php",
      "Cart.php",
      "Taxi.php",
    ]);
  });

  it("reads :N on the end as the line to open at", async () => {
    found = [{ path: "app/Tax.php", score: 10, indices: [] }];
    const w = await show("files");
    await w.find('.search input[type="text"]').setValue("tax:42");
    await wait(80);
    await flushPromises();
    expect(
      asked.mock.calls.find(([cmd]) => cmd === "find_files")![1],
    ).toMatchObject({ query: "tax" });
    press(w, "Enter");
    const tab = tabs.state.tabs.find((one) => one.path === "app/Tax.php")!;
    expect(tab.line).toBe(42);
    expect(tab.mode).toBe("file");
  });
});

describe("the window itself", () => {
  it("has no title row and no close button, only the box and the modes", async () => {
    const w = await show();
    expect(document.querySelector(".modal .head")).toBeNull();
    expect(document.querySelector(".modal h2")).toBeNull();
    expect(document.querySelector(".scrim.palette")).not.toBeNull();
    expect(w.findAll(".mode").map((one) => one.text())).toEqual([
      "In files",
      "Names",
      "Symbols",
    ]);
    // The options sit on the same row as the box.
    expect(w.find('.top .field [data-option="case"]').exists()).toBe(true);
  });

  it("closes on a click on the scrim", async () => {
    await show();
    const scrim = document.querySelector(".scrim") as HTMLElement;
    scrim.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    scrim.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(search.open).toBe(false);
  });

  it("walks the modes with Tab, and back with ⇧Tab, taking the text along", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("Cart");
    press(w, "Tab");
    await flushPromises();
    expect(search.mode).toBe("files");
    expect(search.files.query).toBe("Cart");
    press(w, "Tab");
    await flushPromises();
    expect(search.mode).toBe("symbols");
    press(w, "Tab", { shiftKey: true });
    await flushPromises();
    expect(search.mode).toBe("files");
  });

  it("flips case, word and regex with ⌥C ⌥W ⌥R by the key, whatever it would type", async () => {
    const w = await show();
    // On a Mac ⌥C types ç: the key is read from event.code, and nothing is typed.
    const event = new KeyboardEvent("keydown", {
      key: "ç",
      code: "KeyC",
      altKey: true,
      bubbles: true,
      cancelable: true,
    });
    w.find('.search input[type="text"]').element.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(search.text.case).toBe(true);
    press(w, "∑", { code: "KeyW", altKey: true });
    press(w, "®", { code: "KeyR", altKey: true });
    expect(search.text.word).toBe(true);
    expect(search.text.regex).toBe(true);
    press(w, "ç", { code: "KeyC", altKey: true });
    expect(search.text.case).toBe(false);
    expect(w.find('[data-option="word"]').attributes("title")).toContain("W)");
  });

  it("folds a file under its heading with ← and opens it with →, with the count on it", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("total");
    await wait(150);
    await flushPromises();
    answer([
      {
        path: "app/Cart.php",
        lines: [
          [4, "$total", [[1, 6]]],
          [9, "total", [[0, 5]]],
        ],
      },
      { path: "app/Order.php", lines: [[2, "total", [[0, 5]]]] },
    ]);
    await flushPromises();
    expect(w.findAll(".group .count").map((one) => one.text())).toEqual([
      "2",
      "1",
    ]);
    const input = w.find('.search input[type="text"]')
      .element as HTMLInputElement;
    input.setSelectionRange(0, 0);
    press(w, "ArrowLeft");
    await flushPromises();
    expect(w.findAll(".item")).toHaveLength(1);
    expect(w.find(".group.on").attributes("data-path")).toBe("app/Cart.php");
    // → with the caret before the end of the box moves the caret, nothing else.
    press(w, "ArrowRight");
    await flushPromises();
    expect(w.findAll(".item")).toHaveLength(1);
    input.setSelectionRange(input.value.length, input.value.length);
    press(w, "ArrowRight");
    await flushPromises();
    expect(w.findAll(".item")).toHaveLength(3);
  });

  it("marks every match of the file in the preview", async () => {
    const w = await show();
    await w.find('.search input[type="text"]').setValue("b");
    await wait(150);
    await flushPromises();
    answer([
      {
        path: "app/Cart.php",
        lines: [
          [2, "b", [[0, 1]]],
          [3, "c", []],
        ],
      },
    ]);
    await flushPromises();
    await wait(10);
    await flushPromises();
    expect(
      document.querySelectorAll(".preview mark.find-hit").length,
    ).toBeGreaterThan(0);
    expect(document.querySelector(".where")!.textContent).toContain("Cart.php");
    expect(document.querySelector(".where")!.textContent).toContain(":2");
  });

  it("copies path:line for the result under the cursor on ⌘C", async () => {
    const copied: string[] = [];
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: async (text: string) => void copied.push(text) },
      configurable: true,
    });
    const w = await show();
    await w.find('.search input[type="text"]').setValue("total");
    await wait(150);
    await flushPromises();
    answer([{ path: "app/Cart.php", lines: [[4, "$total", [[1, 6]]]] }]);
    await flushPromises();
    const input = w.find('.search input[type="text"]')
      .element as HTMLInputElement;
    input.setSelectionRange(5, 5);
    press(w, "c", { code: "KeyC", metaKey: isMac, ctrlKey: !isMac });
    await flushPromises();
    expect(copied).toEqual(["app/Cart.php:4"]);
  });
});

describe("finding symbols and shaped file names", () => {
  it("finds a class by its name and opens it at the definition", async () => {
    symbolsFound = [
      {
        name: "MakeExactConnectionCommand",
        kind: "class",
        path: "app/Console/Make.php",
        line: 12,
        column: 6,
        indices: [0, 4, 9, 19],
      },
    ];
    const w = await show("text");
    press(w, "o", {
      code: "KeyO",
      altKey: true,
      metaKey: isMac,
      ctrlKey: !isMac,
    });
    await flushPromises();
    expect(search.mode).toBe("symbols");
    await w.find('.search input[type="text"]').setValue("MECC");
    await wait(80);
    await flushPromises();
    expect(
      asked.mock.calls.find(([cmd]) => cmd === "find_symbols")![1],
    ).toMatchObject({ query: "MECC" });
    expect(w.find(".item .kind").text()).toBe("class");
    expect(w.findAll(".item .symbol b").map((one) => one.text())).toEqual([
      "M",
      "E",
      "C",
      "C",
    ]);
    press(w, "Enter");
    const tab = tabs.state.tabs.find(
      (one) => one.path === "app/Console/Make.php",
    )!;
    expect(tab.line).toBe(12);
    expect(tab.mode).toBe("file");
  });

  it("marks the letters a folder/name query matched in the folder and in the name", async () => {
    const path = "app/Console/Commands/Exact/MakeExactConnectionCommand.php";
    const exact = path.indexOf("Exact/");
    const name = path.lastIndexOf("/") + 1;
    found = [
      {
        path,
        score: 1,
        indices: [0, 1, 2, 3, 4]
          .map((k) => exact + k)
          .concat([0, 1, 2, 3].map((k) => name + k)),
      },
    ];
    const w = await show("files");
    await w.find('.search input[type="text"]').setValue("exact/make");
    await wait(80);
    await flushPromises();
    expect(w.find(".item .file b").text()).toBe("Make");
    expect(w.find(".item .dir b").text()).toBe("Exact");
  });
});
