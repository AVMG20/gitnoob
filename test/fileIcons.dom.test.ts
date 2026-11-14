// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import FileTree from "~/components/FileTree.vue";
import FileList from "~/components/FileList.vue";
import SearchModal from "~/components/SearchModal.vue";
import AppModal from "~/components/AppModal.vue";
import { useGit, type RepoInfo } from "~/composables/useGit";
import { useConfig } from "~/composables/useConfig";
import { resetTabs, switchTabs } from "~/composables/useTabs";
import { resetFileTree } from "~/composables/useFileTree";
import { openSearch, resetSearch } from "~/composables/useSearch";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

const asked = vi.mocked(invoke);
const git = useGit();
const config = useConfig();
let iconsOn = true;
let wrapper: ReturnType<typeof mount> | null = null;

beforeEach(async () => {
  resetTabs();
  resetFileTree();
  resetSearch();
  asked.mockReset();
  asked.mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "config_get")
        return {
          version: 1,
          active_profile: null,
          global: { file_icons: iconsOn },
          profiles: [],
        };
      if (cmd === "list_dir" && args?.path === "") {
        return [
          {
            name: "vendor",
            path: "vendor",
            dir: true,
            ignored: true,
            link: false,
          },
          {
            name: "app.ts",
            path: "app.ts",
            dir: false,
            ignored: false,
            link: false,
          },
          {
            name: "home.blade.php",
            path: "home.blade.php",
            dir: false,
            ignored: false,
            link: false,
          },
        ];
      }
      if (cmd === "find_files")
        return [{ path: "app/Cart.php", score: 9, indices: [] }];
      return null;
    },
  );
  git.store.repo = {
    path: "/repos/shop",
    name: "shop",
    head: "main",
  } as RepoInfo;
  git.store.status = {
    staged: [],
    unstaged: [{ path: "app.ts", kind: "modified" }],
    conflicted: [],
  } as never;
  switchTabs("/repos/shop");
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  iconsOn = true;
  git.store.repo = null;
  git.store.status = null;
});

async function settle(on: boolean) {
  iconsOn = on;
  await config.load();
}

const glyphOf = (w: ReturnType<typeof mount>, selector: string) =>
  w.find(`${selector} .file-icon`).attributes("class");

describe("file icons in the lists", () => {
  it("the Files tree draws one per row, by type, with the status left on the name", async () => {
    await settle(true);
    wrapper = mount(FileTree, {
      attachTo: document.body,
      global: { components: { AppModal } },
    });
    await flushPromises();
    expect(glyphOf(wrapper, '.row[data-path="app.ts"]')).toContain("t-ts");
    expect(glyphOf(wrapper, '.row[data-path="home.blade.php"]')).toContain(
      "t-blade",
    );
    expect(glyphOf(wrapper, '.row[data-path="vendor"]')).toContain(
      "t-folder-composer",
    );
    // Modified colours the row's name; the icon keeps its own colour class.
    expect(wrapper.find('.row[data-path="app.ts"]').classes()).toContain(
      "modified",
    );
  });

  it("the right panel draws one before each name and a folder icon for each folder", async () => {
    await settle(true);
    wrapper = mount(FileList, {
      props: {
        files: [
          { path: "src/main.rs", kind: "modified" },
          { path: "src/lib.rs", kind: "added" },
        ],
      },
    });
    await flushPromises();
    const icons = wrapper.findAll(".file-icon");
    expect(icons.some((one) => one.classes().includes("t-rust"))).toBe(true);
    expect(wrapper.find(".row.dir .file-icon").exists()).toBe(true);
  });

  it("the search window draws them on the file names", async () => {
    await settle(true);
    openSearch("files");
    wrapper = mount(SearchModal, {
      attachTo: document.body,
      global: { components: { AppModal } },
    });
    await flushPromises();
    const input = wrapper.find('.search input[type="text"]');
    await input.setValue("cart");
    await new Promise((done) => setTimeout(done, 200));
    await flushPromises();
    expect(document.querySelector(".item .file-icon.t-php")).not.toBeNull();
  });

  it("the setting hides them everywhere", async () => {
    await settle(false);
    wrapper = mount(FileTree, {
      attachTo: document.body,
      global: { components: { AppModal } },
    });
    await flushPromises();
    expect(wrapper.find('.row[data-path="app.ts"]').exists()).toBe(true);
    expect(wrapper.find(".file-icon").exists()).toBe(false);
    wrapper.unmount();
    wrapper = mount(FileList, {
      props: { files: [{ path: "src/main.rs", kind: "modified" }] },
    });
    await flushPromises();
    expect(wrapper.find(".file-icon").exists()).toBe(false);
    // Folders go back to the plain folder they had before icons.
    expect(wrapper.find(".row.dir .folder").exists()).toBe(true);
  });
});
