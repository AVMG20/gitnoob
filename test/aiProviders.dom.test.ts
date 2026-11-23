// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SettingsModal from '~/components/SettingsModal.vue'
import AppModal from '~/components/AppModal.vue'
import { useConfig, type AiProvider } from '~/composables/useConfig'
import { useAi } from '~/composables/useAi'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const config = useConfig()
const ai = useAi()

let state = {
  provider: 'openrouter' as AiProvider,
  model: 'anthropic/claude-sonnet-4.5' as string | null,
  claude_model: 'haiku' as string | null,
  claude_effort: null as string | null,
  codex_model: null as string | null,
  codex_effort: null as string | null,
  antigravity_model: null as string | null,
  antigravity_effort: null as string | null
}

let modelCalls: { provider: string; refresh: boolean }[] = []

function makeConfig() {
  return {
    version: 1,
    active_profile: null,
    global: {
      show_avatars: true,
      ai: {
        provider: state.provider,
        model: state.model,
        claude_model: state.claude_model,
        claude_effort: state.claude_effort,
        codex_model: state.codex_model,
        codex_effort: state.codex_effort,
        antigravity_model: state.antigravity_model,
        antigravity_effort: state.antigravity_effort,
        max_tokens: 3000,
        reasoning: 'off',
        commit_style: 'plain',
        commit_prompt: null
      }
    },
    profiles: []
  }
}

beforeEach(async () => {
  state = {
    provider: 'openrouter',
    model: 'anthropic/claude-sonnet-4.5',
    claude_model: 'haiku',
    claude_effort: null,
    codex_model: null,
    codex_effort: null,
    antigravity_model: null,
    antigravity_effort: null
  }
  modelCalls = []
  ai.store.cliInfo = []
  ai.store.cliModels = {}
  ai.store.loadingCliModels = {}
  ai.store.cliModelsError = {}
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'config_set_global') {
      const next = (args?.global as { ai: typeof state }).ai
      state = {
        provider: next.provider ?? state.provider,
        model: next.model ?? null,
        claude_model: next.claude_model ?? 'haiku',
        claude_effort: next.claude_effort ?? null,
        codex_model: next.codex_model ?? null,
        codex_effort: next.codex_effort ?? null,
        antigravity_model: next.antigravity_model ?? null,
        antigravity_effort: next.antigravity_effort ?? null
      }
      return makeConfig()
    }
    if (cmd === 'config_get') return makeConfig()
    if (cmd === 'ai_status') {
      const activeModel =
        state.provider === 'openrouter'
          ? state.model
          : state.provider === 'claude'
            ? state.claude_model || 'haiku'
            : state.provider === 'codex'
              ? state.codex_model
              : state.antigravity_model
      return {
        configured: true,
        provider: state.provider,
        model: activeModel,
        default_commit_prompt: 'Write a short commit message.'
      }
    }
    if (cmd === 'ai_cli_info') {
      return [
        { provider: 'claude', binary: 'claude', available: true, path: '/usr/local/bin/claude', version: '2.1.0' },
        { provider: 'codex', binary: 'codex', available: true, path: '/usr/local/bin/codex', version: '0.169.0' },
        { provider: 'antigravity', binary: 'agy', available: true, path: '/usr/local/bin/agy', version: '1.5.14' }
      ]
    }
    if (cmd === 'ai_cli_models') {
      const provider = String(args?.provider ?? '')
      modelCalls.push({ provider, refresh: Boolean(args?.refresh) })
      if (provider === 'claude') {
        return [
          { id: 'haiku', name: 'Haiku 5.5', note: 'Fastest for quick answers', effort_levels: ['low', 'medium', 'high'] },
          { id: 'sonnet', name: 'Sonnet 5.5', note: 'Balanced', effort_levels: ['low', 'medium', 'high', 'max'] }
        ]
      }
      if (provider === 'codex') {
        return [
          { id: 'gpt-6-luna', name: 'GPT-6-Luna', note: 'Fast coding model', effort_levels: ['low', 'medium', 'high'] },
          { id: 'gpt-5.6-terra', name: 'GPT-5.6-Terra', note: 'Balanced', effort_levels: ['low', 'medium', 'high', 'ultra'] }
        ]
      }
      if (provider === 'antigravity') {
        return [
          { id: 'gemini-3.8-flash', name: 'Gemini 3.8 Flash', note: '', effort_levels: ['low', 'medium', 'high'], default_effort: 'low' },
          { id: 'gemini-4-neon', name: 'Gemini 4 Neon', note: '', effort_levels: ['low', 'medium', 'high', 'xhigh'], default_effort: 'low' }
        ]
      }
      return []
    }
    if (cmd === 'ai_models') return []
    return null
  })
  config.store.settingsSection = 'ai'
  await config.load()
  await ai.refreshStatus()
})

async function open() {
  const wrapper = mount(SettingsModal, { global: { components: { AppModal } } })
  await flushPromises()
  return wrapper
}

describe('AI method selection in Settings', () => {
  it('shows OpenRouter controls only when OpenRouter is selected', async () => {
    const wrapper = await open()
    expect(wrapper.find('input.max-tokens').exists()).toBe(true)
    expect(wrapper.find('[data-testid="cli-status"]').exists()).toBe(false)

    await wrapper.find('[data-provider="claude"]').trigger('click')
    await flushPromises()

    expect(state.provider).toBe('claude')
    expect(wrapper.find('input.max-tokens').exists()).toBe(false)
    expect(wrapper.find('[data-testid="cli-status"]').text()).toContain('Claude CLI is installed')
    expect(wrapper.find('[data-testid="cli-models"]').text()).toContain('Haiku 5.5')
    expect(modelCalls).toContainEqual({ provider: 'claude', refresh: false })
  })

  it('defaults Claude CLI to haiku, selects thinking mode, and refreshes models on Recheck', async () => {
    state.provider = 'claude'
    await config.load()
    const wrapper = await open()

    expect(wrapper.find('[data-model="haiku"]').classes()).toContain('on')
    expect(wrapper.find('input[type="text"]').exists()).toBe(false)

    await wrapper.find('[data-model="sonnet"]').trigger('click')
    await flushPromises()
    expect(state.claude_model).toBe('sonnet')

    await wrapper.find('[data-effort="high"]').trigger('click')
    await flushPromises()
    expect(state.claude_effort).toBe('high')
    expect(wrapper.find('[data-effort="high"]').classes()).toContain('on')

    await wrapper.find('[data-testid="cli-recheck"]').trigger('click')
    await flushPromises()
    expect(modelCalls).toContainEqual({ provider: 'claude', refresh: true })
  })

  it('lists live Codex models and updates available thinking levels when picking a model', async () => {
    const wrapper = await open()
    await wrapper.find('[data-provider="codex"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-testid="cli-models"]').text()).toContain('GPT-6-Luna')
    expect(wrapper.find('[data-testid="cli-models"]').text()).toContain('GPT-5.6-Terra')

    await wrapper.find('[data-model="gpt-5.6-terra"]').trigger('click')
    await flushPromises()
    expect(state.codex_model).toBe('gpt-5.6-terra')
    expect(wrapper.find('[data-effort="ultra"]').exists()).toBe(true)

    await wrapper.find('[data-effort="ultra"]').trigger('click')
    await flushPromises()
    expect(state.codex_effort).toBe('ultra')

    // Switching to a model without `ultra` resets effort back to default.
    await wrapper.find('[data-model="gpt-6-luna"]').trigger('click')
    await flushPromises()
    expect(state.codex_model).toBe('gpt-6-luna')
    expect(state.codex_effort).toBeNull()
  })

  it('lists live Antigravity models and allows selecting model and thinking mode', async () => {
    const wrapper = await open()
    await wrapper.find('[data-provider="antigravity"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-testid="cli-models"]').text()).toContain('Gemini 3.8 Flash')
    expect(wrapper.find('[data-testid="cli-models"]').text()).toContain('Gemini 4 Neon')

    await wrapper.find('[data-model="gemini-4-neon"]').trigger('click')
    await flushPromises()
    expect(state.antigravity_model).toBe('gemini-4-neon')

    await wrapper.find('[data-effort="xhigh"]').trigger('click')
    await flushPromises()
    expect(state.antigravity_effort).toBe('xhigh')
  })
})
