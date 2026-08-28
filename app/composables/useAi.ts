import { invoke } from './useInvoke'
import { computed, reactive } from 'vue'

export interface Model {
  id: string
  name: string
  context_length: number
  /** US dollars per million tokens. */
  prompt_price: number
  completion_price: number
  description: string
  multimodal: boolean
}

export interface AiStatus {
  configured: boolean
  model: string | null
  /** What the settings box starts from, sent by the backend that owns it. */
  default_commit_prompt: string
}

export interface CommitMessage {
  summary: string
  body: string
}

const store = reactive({
  status: { configured: false, model: null, default_commit_prompt: '' } as AiStatus,
  models: [] as Model[],
  loadingModels: false,
  modelsError: null as string | null,
  /** Set while a request is in flight, so buttons can show what is happening. */
  busy: null as string | null
})

export function useAi() {
  const configured = computed(() => store.status.configured)

  async function refreshStatus() {
    store.status = await invoke<AiStatus>('ai_status').catch(() => store.status)
  }

  async function loadModels(refresh = false) {
    if (store.loadingModels) return
    store.loadingModels = true
    store.modelsError = null
    try {
      store.models = await invoke<Model[]>('ai_models', { refresh })
    } catch (error) {
      store.modelsError = String(error)
    } finally {
      store.loadingModels = false
    }
  }

  async function run<T>(label: string, fn: () => Promise<T>): Promise<T | null> {
    store.busy = label
    try {
      return await fn()
    } finally {
      store.busy = null
    }
  }

  return {
    store,
    configured,
    refreshStatus,
    loadModels,
    commitMessage: () =>
      run('commit message', () => invoke<CommitMessage>('ai_commit_message')),
    /** A message for a commit that already exists, written from its own diff. */
    commitMessageFor: (oid: string) =>
      run('commit message', () => invoke<CommitMessage>('ai_commit_message_for', { oid })),
    /** One message for the commits a squash is about to fold into one. */
    squashMessage: (oids: string[]) =>
      run('commit message', () => invoke<CommitMessage>('ai_squash_message', { oids })),
    /** A review's title and description, written from the branch's commits. */
    reviewMessage: (source: string, target: string) =>
      run('review description', () =>
        invoke<CommitMessage>('ai_review_message', { source, target })
      ),
    resolveConflict: (path: string, index: number) =>
      run('conflict', () => invoke<string[]>('ai_resolve_conflict', { path, index }))
  }
}

/** Prices come per million tokens; show them the way OpenRouter quotes them. */
export function priceLabel(model: Model) {
  const format = (value: number) => {
    if (value === 0) return 'free'
    if (value < 1) return `$${value.toFixed(3)}`
    return `$${value.toFixed(2)}`
  }
  return `${format(model.prompt_price)} in · ${format(model.completion_price)} out`
}

export function contextLabel(model: Model) {
  if (!model.context_length) return ''
  if (model.context_length >= 1_000_000) return `${(model.context_length / 1_000_000).toFixed(1)}M`
  return `${Math.round(model.context_length / 1000)}K`
}
