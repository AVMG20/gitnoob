// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import SettingsModal from "~/components/SettingsModal.vue";
import AppModal from "~/components/AppModal.vue";
import { useConfig } from "~/composables/useConfig";
import { useAi } from "~/composables/useAi";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const asked = vi.mocked(invoke);
const config = useConfig();
const ai = useAi();
let saved: number[] = [];
let current = 3000;

function stored(maxTokens: number) {
  return {
    version: 1,
    active_profile: null,
    global: {
      show_avatars: true,
      ai: {
        model: "a/b",
        max_tokens: maxTokens,
        reasoning: "off",
        commit_style: "plain",
        commit_prompt: null,
      },
    },
    profiles: [],
  };
}

beforeEach(async () => {
  saved = [];
  current = 3000;
  asked.mockReset();
  asked.mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "config_set_global") {
        const global = args?.global as { ai: { max_tokens: number } };
        current = global.ai.max_tokens;
        saved.push(current);
      }
      if (cmd === "config_get" || cmd === "config_set_global")
        return stored(current);
      if (cmd === "ai_status")
        return { configured: true, model: "a/b", default_commit_prompt: "" };
      if (cmd === "ai_models") return [];
      return null;
    },
  );
  config.store.settingsSection = "ai";
  await config.load();
  await ai.refreshStatus();
});

async function open() {
  const wrapper = mount(SettingsModal, {
    global: { components: { AppModal } },
  });
  await flushPromises();
  return wrapper;
}

const box = (wrapper: Awaited<ReturnType<typeof open>>) =>
  wrapper.find("input.max-tokens");

async function type(wrapper: Awaited<ReturnType<typeof open>>, value: string) {
  const input = box(wrapper);
  (input.element as HTMLInputElement).value = value;
  await input.trigger("change");
  await flushPromises();
}

describe("the token cap", () => {
  it("is shown with the stored number", async () => {
    const wrapper = await open();
    expect((box(wrapper).element as HTMLInputElement).value).toBe("3000");
  });

  it("is saved when changed", async () => {
    const wrapper = await open();
    await type(wrapper, "8000");
    expect(saved).toEqual([8000]);
  });

  it("is kept inside a range a request can be made with", async () => {
    const wrapper = await open();
    await type(wrapper, "5");
    expect(saved).toEqual([256]);
    await type(wrapper, "9999999");
    expect(saved).toEqual([256, 200000]);
  });

  it("ignores a blank or unchanged value", async () => {
    const wrapper = await open();
    await type(wrapper, "");
    await type(wrapper, "3000");
    expect(saved).toEqual([]);
    expect((box(wrapper).element as HTMLInputElement).value).toBe("3000");
  });
});
