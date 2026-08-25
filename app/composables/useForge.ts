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

export interface ForgeUser {
  login: string
  /** The picture as a `data:` URL, or null when the forge has none. */
  avatar: string | null
}

const store = reactive({
  status: null as ForgeStatus | null,
  reviews: [] as Review[],
  loading: false,
  error: null as string | null,
  checking: false,
  /** Who the active profile's token belongs to, once asked. */
  me: null as ForgeUser | null,
  /** The profile `me` describes, so a switch does not show the last face. */
  meFor: null as string | null,
  /** Every profile's picture, by profile id, for the switcher. */
  faces: {} as Record<string, string>
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
    await loadMe()
  }

  /**
   * Asks every profile's forge for its picture.
   *
   * Kept apart from `loadMe` because it is about the other profiles: the
   * switcher shows accounts, and an account is recognised by its face before
   * its name is read. Cached for the run on the other side, so calling this
   * whenever the menu opens costs nothing after the first time.
   */
  async function loadFaces() {
    store.faces = await invoke<Record<string, string>>('forge_faces').catch(() => ({}))
  }

  /**
   * Asks the forge who the token belongs to, once per profile.
   *
   * The answer is only ever decoration — a name and a face on the profile
   * menu — so a forge that is down, or a token that has expired, simply leaves
   * the icon where it was. The failure that matters is reported by the
   * connection test, where the user is asking about it.
   */
  async function loadMe(force = false) {
    const id = profileId()
    if (!force && store.meFor === id) return
    store.meFor = id
    store.me = null
    if (!id || !store.status?.has_token || store.status.kind === 'none') return
    store.me = await invoke<ForgeUser>('forge_me').catch(() => null)
  }

  /** The profile a lookup belongs to: its forge and host, since either changing
      means a different account. */
  function profileId() {
    if (!store.status || store.status.kind === 'none') return null
    return `${store.status.kind}@${store.status.host}`
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
      // A token that has just been proved good is worth asking about again:
      // this is the moment a freshly pasted one gets its face.
      await loadMe(true)
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
    loadFaces,
    loadReviews,
    loadMe,
    check,
    createReview: (title: string, body: string, target: string, draft: boolean) =>
      invoke<Review>('forge_create_review', { title, body, target, draft }),
    open: (url: string) => invoke('open_external', { url }),
    /** The forge's token-creation page, with scopes and a name pre-filled. */
    tokenUrl: (kind: ForgeKind, host: string) =>
      invoke<string | null>('forge_token_url', { kind, host })
  }
}
