// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import FileEditor from "~/components/FileEditor.vue";
import DiffViewer from "~/components/DiffViewer.vue";
import FileView from "~/components/FileView.vue";
import AppModal from "~/components/AppModal.vue";
import { useGit } from "~/composables/useGit";
import { diffMode } from "~/composables/useDiffMode";
import { openTab, resetTabs, switchTabs, tabId } from "~/composables/useTabs";
import { resetEdits } from "~/composables/useFileEdit";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(async () => true) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

const asked = vi.mocked(invoke);
const git = useGit();
const PATH = "app/Tax.php";
const TEXT = "<?php\nreturn 21;\n$rate = 2;\n";
let calls: { cmd: string; args: Record<string, unknown> }[] = [];
let readonly: string | null = null;
let wrapper: ReturnType<typeof mount> | null = null;

/** Line 1 changed (was `<?hh`), line 3 added. */
const HUNKS = [
  {
    header: "@@ -1,2 +1,3 @@",
    lines: [
      { origin: "-", old_lineno: 1, new_lineno: null, content: "<?hh" },
      { origin: "+", old_lineno: null, new_lineno: 1, content: "<?php" },
      { origin: " ", old_lineno: 2, new_lineno: 2, content: "return 21;" },
      { origin: "+", old_lineno: null, new_lineno: 3, content: "$rate = 2;" },
    ],
  },
];

beforeEach(() => {
  resetTabs();
  resetEdits();
  switchTabs("/repos/shop");
  git.store.repo = { path: "/repos/shop", name: "shop", head: "main" } as never;
  openTab({ path: PATH, mode: "file" });
  calls = [];
  readonly = null;
  asked.mockReset();
  asked.mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} });
      if (cmd === "read_file") {
        return {
          text: TEXT,
          hash: "h1",
          eol: "lf",
          bom: false,
          readonly,
          missing: false,
        };
      }
      if (cmd === "working_file_diff" || cmd === "head_file_diff") {
        return { path: PATH, binary: false, truncated: 0, hunks: HUNKS };
      }
      if (cmd === "file_text") return TEXT;
      if (cmd === "write_file") return "h2";
      if (cmd === "config_get") return { global: {}, profiles: [] };
      if (cmd === "unused_in" || cmd === "usage_counts") return [];
      return null;
    },
  );
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  git.store.repo = null;
  git.store.viewer = null;
  git.store.status = null;
  diffMode.mode = "diff";
});

async function show(props: Record<string, unknown> = {}) {
  wrapper = mount(FileEditor, {
    attachTo: document.body,
    props: {
      tabId: tabId(PATH),
      path: PATH,
      hunks: HUNKS,
      runs: null,
      blame: false,
      find: null,
      fallback: null,
      ...props,
    },
    global: { components: { AppModal } },
  });
  await flushPromises();
  return wrapper;
}

const editorOf = (w: ReturnType<typeof mount>) =>
  EditorView.findFromDOM(w.find(".host .cm-editor").element as HTMLElement)!;

/** happy-dom lays nothing out: a click on the bar gutter lands on line 1. */
async function clickBar(w: ReturnType<typeof mount>) {
  const bar = w.find(".cm-bar");
  expect(bar.attributes("title")).toMatch(/click to see what was here/i);
  bar.element.parentElement!.dispatchEvent(
    new MouseEvent("mousedown", { bubbles: true, clientY: 0 }),
  );
  await flushPromises();
}

describe("the card a change bar opens", () => {
  it("opens under the change and leaves the tab in File mode", async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: PATH, kind: "modified" }],
      conflicted: [],
    } as never;
    git.store.viewer = { path: PATH, side: "unstaged" } as never;
    diffMode.mode = "file";
    wrapper = mount(DiffViewer, {
      attachTo: document.body,
      global: {
        components: { FileView, FileEditor },
        stubs: { Spinner: true, ChangeRuler: true },
      },
    });
    await flushPromises();
    await clickBar(wrapper);
    expect(wrapper.find(".change-card").exists()).toBe(true);
    expect(wrapper.find(".change-card .title").text()).toBe("Change 1 of 2");
    expect(wrapper.find(".change-card .was").text()).toContain("<?hh");
    expect(diffMode.mode).toBe("file");
    // Esc closes the card, not the file.
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        cancelable: true,
      }),
    );
    await flushPromises();
    expect(wrapper.find(".change-card").exists()).toBe(false);
    expect(git.store.viewer).not.toBeNull();
  });

  it("reverts the change in the buffer as one step ⌘Z takes back", async () => {
    const w = await show();
    await clickBar(w);
    await w.find('[data-act="revert"]').trigger("click");
    const view = editorOf(w);
    expect(view.state.doc.toString()).toBe("<?hh\nreturn 21;\n$rate = 2;\n");
    expect(w.find(".change-card").exists()).toBe(false);
    // No git call: the buffer changed, and the usual save follows.
    expect(
      calls.some((one) => one.cmd === "apply_hunk" || one.cmd === "discard"),
    ).toBe(false);
    undo(view);
    expect(view.state.doc.toString()).toBe(TEXT);
  });

  it("stages just that change through the hunk-staging command", async () => {
    const w = await show();
    await clickBar(w);
    await w.find('[data-act="stage"]').trigger("click");
    await flushPromises();
    const call = calls.find((one) => one.cmd === "apply_hunk")!;
    expect(call.args).toMatchObject({
      path: PATH,
      hunkIndex: 0,
      action: "stage",
      lines: { added: [1], removed: [1] },
    });
  });

  it("moves from change to change with its arrows and with ⌥↓, wrapping round", async () => {
    const w = await show();
    await clickBar(w);
    const buttons = w.findAll(".change-card .icon");
    await buttons[1]!.trigger("click");
    expect(w.find(".change-card .title").text()).toBe("Change 2 of 2");
    expect(w.find(".change-card .note").text()).toBe(
      "Added lines; nothing was here before.",
    );
    // file.nextChange reaches the editor as nextChange while the card is open.
    (w.vm as unknown as { nextChange: (by: number) => void }).nextChange(1);
    await flushPromises();
    expect(w.find(".change-card .title").text()).toBe("Change 1 of 2");
    await buttons[0]!.trigger("click");
    expect(w.find(".change-card .title").text()).toBe("Change 2 of 2");
  });

  it("shows a read-only file the old lines without Revert or Stage", async () => {
    readonly = "endings";
    const w = await show();
    await clickBar(w);
    expect(w.find(".change-card").exists()).toBe(true);
    expect(w.find('[data-act="revert"]').exists()).toBe(false);
    expect(w.find('[data-act="stage"]').exists()).toBe(false);
    expect(w.find('[data-act="copy"]').exists()).toBe(true);
  });

  it("closes on a click outside it, and stays while typing elsewhere", async () => {
    const w = await show();
    await clickBar(w);
    editorOf(w).dispatch({
      changes: { from: TEXT.length, insert: "x" },
      userEvent: "input.type",
    });
    await flushPromises();
    expect(w.find(".change-card").exists()).toBe(true);
    document.body.dispatchEvent(
      new PointerEvent("pointerdown", { bubbles: true }),
    );
    await flushPromises();
    expect(w.find(".change-card").exists()).toBe(false);
  });
});
