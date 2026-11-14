// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { EditorView } from "@codemirror/view";
import DiffViewer from "~/components/DiffViewer.vue";
import FileView from "~/components/FileView.vue";
import FileEditor from "~/components/FileEditor.vue";
import { diffMode } from "~/composables/useDiffMode";
import { useGit } from "~/composables/useGit";
import { deletingFiles } from "~/composables/useFileView";
import { resetEdits, setSaveError, setSaveState } from "~/composables/useFileEdit";
import { aimedAt } from "~/composables/useInvoke";
import { resetTabs, switchTabs, tabId } from "~/composables/useTabs";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

const asked = vi.mocked(invoke);
const git = useGit();
let calls: string[] = [];
let ignoredAnswer = false;
let open: ReturnType<typeof mount> | null = null;

beforeEach(() => {
  resetTabs();
  resetEdits();
  switchTabs("/repo");
  calls = [];
  ignoredAnswer = false;
  deletingFiles.value = null;
  asked.mockReset();
  asked.mockImplementation(async (cmd: string) => {
    calls.push(cmd);
    if (cmd === "file_text") return "one\ntwo\n";
    if (cmd === "read_file") {
      return {
        text: "one\ntwo\n",
        hash: "h1",
        eol: "lf",
        bom: false,
        readonly: null,
        missing: false,
      };
    }
    if (cmd === "write_file") return "h2";
    if (cmd === "working_file_diff" || cmd === "head_file_diff") {
      return { path: "app/Cart.php", binary: false, truncated: 0, hunks: [] };
    }
    if (cmd === "path_ignored") return ignoredAnswer;
    return null;
  });
  git.store.repo = {
    path: "/repo",
    name: "repo",
    head: "main",
    detached: false,
  } as never;
  git.store.viewer = { path: "app/Cart.php", side: "unstaged" } as never;
  diffMode.mode = "file";
});

afterEach(() => {
  open?.unmount();
  open = null;
  diffMode.mode = "diff";
  git.store.status = null;
});

async function show() {
  open = mount(DiffViewer, {
    attachTo: document.body,
    global: {
      components: { FileView, FileEditor },
      stubs: { Spinner: true, ChangeRuler: true },
    },
  });
  await flushPromises();
  return open;
}

const labels = (w: ReturnType<typeof mount>) =>
  w.findAll(".bar .btn").map((one) => one.text());

describe("the bar over an open file", () => {
  it("gives an unchanged file no Stage or Discard, and no Diff to switch to", async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: "other.txt", kind: "modified" }],
      conflicted: [],
    } as never;
    const wrapper = await show();
    expect(labels(wrapper).some((one) => /Stage|Discard/.test(one))).toBe(
      false,
    );
    expect(wrapper.find(".stats").exists()).toBe(false);
    expect(
      wrapper
        .findAll(".seg")
        .every((seg) => seg.attributes("disabled") !== undefined),
    ).toBe(true);
    // Read, not reviewed: the browsing tools are there.
    expect(
      wrapper
        .find('.bar [title="Every commit that touched this file"]')
        .exists(),
    ).toBe(true);
    expect(
      wrapper.find('.bar [title="Show it in the Files tree"]').exists(),
    ).toBe(true);
    // Tab does nothing with nothing to compare.
    window.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Tab", bubbles: true }),
    );
    expect(diffMode.mode).toBe("file");
    // Saving, saved and unsaved are the tab's dot, not words in the bar.
    expect(wrapper.find(".save-warning").exists()).toBe(false);
    expect(wrapper.find(".bar").text()).not.toMatch(/Saved|Saving|Unsaved/);
    // The caret and how the file is written.
    expect(wrapper.find(".facts").text()).toContain("Ln 1, Col 1");
    expect(wrapper.find(".facts").text()).toContain("LF");
  });

  it("says only the save states that need doing something about, as a chip", async () => {
    git.store.status = { staged: [], unstaged: [{ path: "app/Cart.php", kind: "modified" }], conflicted: [] } as never;
    const wrapper = await show();
    const id = tabId("app/Cart.php");
    setSaveState(aimedAt(), id, "unsaved");
    await flushPromises();
    expect(wrapper.find(".save-warning").exists()).toBe(false);
    setSaveState(aimedAt(), id, "saving");
    await flushPromises();
    expect(wrapper.find(".bar").text()).not.toContain("Saving");
    setSaveError(aimedAt(), id, "Error: disk full");
    setSaveState(aimedAt(), id, "failed");
    await flushPromises();
    expect(wrapper.find(".save-warning").text()).toBe("Not saved");
    expect(wrapper.find(".save-warning").attributes("title")).toContain("disk full");
    setSaveState(aimedAt(), id, "conflict");
    await flushPromises();
    expect(wrapper.find(".save-warning").text()).toBe("Changed on disk");
  });

  it("calls an untracked file new and offers Add and Delete", async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: "app/Cart.php", kind: "untracked" }],
      conflicted: [],
    } as never;
    const wrapper = await show();
    expect(wrapper.find(".badge.new").text()).toBe("new");
    expect(labels(wrapper)).toEqual(
      expect.arrayContaining(["Add file", "Delete file"]),
    );
    const remove = wrapper
      .findAll(".bar .btn")
      .find((one) => one.text() === "Delete file")!;
    await remove.trigger("click");
    // The right panel's own question, not a delete straight away.
    expect(deletingFiles.value).toEqual(["app/Cart.php"]);
    expect(calls).not.toContain("delete_untracked");
  });

  it("marks an ignored file and offers git nothing", async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: [] } as never;
    ignoredAnswer = true;
    const wrapper = await show();
    expect(wrapper.find(".badge.ignored").exists()).toBe(true);
    expect(labels(wrapper).some((one) => /Stage|Discard|Add/.test(one))).toBe(
      false,
    );
  });

  it("turns to the git buttons once typing shows up in the status", async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: "other.txt", kind: "modified" }],
      conflicted: [],
    } as never;
    const wrapper = await show();
    expect(labels(wrapper).some((one) => one.includes("Stage file"))).toBe(
      false,
    );
    const view = EditorView.findFromDOM(
      wrapper.find(".cm-editor").element as HTMLElement,
    )!;
    view.dispatch({
      changes: { from: 0, insert: "zero\n" },
      userEvent: "input.type",
    });
    await flushPromises();
    git.store.status = {
      staged: [],
      unstaged: [
        { path: "other.txt", kind: "modified" },
        { path: "app/Cart.php", kind: "modified" },
      ],
      conflicted: [],
    } as never;
    await flushPromises();
    expect(labels(wrapper)).toEqual(
      expect.arrayContaining(["Discard", "Stage file"]),
    );
    expect(
      wrapper
        .findAll(".seg")
        .every((seg) => seg.attributes("disabled") === undefined),
    ).toBe(true);
  });
});
