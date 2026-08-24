import { invoke } from '@tauri-apps/api/core'
import { computed, reactive } from 'vue'
import type { ForgeKind } from '~/composables/useConfig'

export interface RepoSlug {
  host: string
  owner: string
  name: string
}

export interface ForgeStatus {
  kind: ForgeKind
  host: string
  has_token: boolean
  user: string | null
  slug: RepoSlug | null
  error: string | null
}

export interface Review {
  number: number
  title: string
  author: string
  state: string
  draft: boolean
  source_branch: string
  target_branch: string
  url: string
  updated_at: string
  is_current: boolean
}

const store = reactive({
  status: null as ForgeStatus | null,
  reviews: [] as Review[],
  loading: false,
  error: null as string | null,
  checking: false
})

export function useForge() {
  /** Usable means: a forge is chosen, a token is stored, and the remote parsed. */
  const usable = computed(
    () =>
      !!store.status &&
      store.status.kind !== 'none' &&
      store.status.has_token &&
      !!store.status.slug
  )
  const label = computed(() => (store.status?.kind === 'gitlab' ? 'Merge requests' : 'Pull requests'))
  const shortLabel = computed(() => (store.status?.kind === 'gitlab' ? 'MR' : 'PR'))

  async function refreshStatus() {
    store.status = await invoke<ForgeStatus>('forge_status').catch(() => null)
  }

  async function loadReviews() {
    if (!usable.value) {
      store.reviews = []
      return
    }
    store.loading = true
    store.error = null
    try {
      store.reviews = await invoke<Review[]>('forge_reviews')
    } catch (error) {
      store.error = String(error)
      store.reviews = []
    } finally {
      store.loading = false
    }
  }

  async function check() {
    store.checking = true
    store.error = null
    try {
      const user = await invoke<string>('forge_check')
      if (store.status) store.status.user = user
      return user
    } catch (error) {
      store.error = String(error)
      return null
    } finally {
      store.checking = false
    }
  }

  return {
    store,
    usable,
    label,
    shortLabel,
    refreshStatus,
    loadReviews,
    check,
    createReview: (title: string, body: string, target: string, draft: boolean) =>
      invoke<Review>('forge_create_review', { title, body, target, draft }),
    open: (url: string) => invoke('open_external', { url }),
    /** The forge's token-creation page, with scopes and a name pre-filled. */
    signinUrl: (kind: ForgeKind, host: string) =>
      invoke<string | null>('forge_signin_url', { kind, host })
  }
}
