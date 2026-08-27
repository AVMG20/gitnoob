import { invoke } from './useInvoke'
import { computed, reactive } from 'vue'
import type { ForgeKind } from '~/composables/useConfig'
import { forgetAvatars } from '~/composables/useAvatars'

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

/** Someone a review can be handed to. */
export interface Member {
  /** GitLab addresses people by number, GitHub by login; both are kept. */
  id: number
  login: string
  name: string
}

/**
 * The repository a review's branch lives in.
 *
 * A review opened from a fork has its branch somewhere this clone has never
 * spoken to, which is why checking one out needs the fork's address as well as
 * the branch's name.
 */
export interface ReviewSource {
  /** `owner/name` on the forge. */
  full_name: string
  owner: string
  ssh_url: string
  https_url: string
  /** False when the branch is in the repository being reviewed. */
  is_fork: boolean
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
  /** The tip of the source branch when the forge was last asked. */
  head_sha: string
  /** Null for a review whose fork has been deleted. */
  source: ReviewSource | null
  /** Set when the review opened but something after it did not. */
  warning: string | null
}

/** Somebody a review names: its author, an assignee, a reviewer. */
export interface Person {
  login: string
  /** Their real name where the forge has one; the login otherwise. */
  name: string
  /** Their picture as a `data:` URL, or null when there was none to fetch. */
  avatar: string | null
}

/** One of a review's labels, with the colour the forge gave it. */
export interface Label {
  name: string
  /** `#rrggbb`, or empty where the forge does not colour its labels. */
  color: string
}

/**
 * Everything one review says about itself.
 *
 * The list in the sidebar is asked for on every refresh and stays thin; this
 * is fetched once, when a particular review is being read.
 */
export interface ReviewDetail {
  number: number
  title: string
  /** The review's own description, which is not the head commit's message. */
  body: string
  state: string
  draft: boolean
  author: Person
  assignees: Person[]
  reviewers: Person[]
  labels: Label[]
  milestone: string | null
  source_branch: string
  target_branch: string
  url: string
  created_at: string
  updated_at: string
  comments: number
  /** Whether it can be merged, in the forge's own vocabulary. */
  merge_status: string | null
  /** The versions being compared, which anchoring a comment to a diff line
      needs naming: GitLab wants all three, GitHub only the head. */
  base_sha: string
  head_sha: string
  start_sha: string
}

export interface ForgeUser {
  login: string
  /** The forge's own numeric id, which GitLab wants instead of the login. */
  id: number
  /** The picture as a `data:` URL, or null when the forge has none. */
  avatar: string | null
}

/** Everything the forge needs to open one. */
export interface NewReview {
  source: string
  target: string
  title: string
  body: string
  draft: boolean
  assignees: Member[]
  reviewers: Member[]
}

/** A repository the token can see, for picking one to clone. */
export interface ForgeRepo {
  name: string
  full_name: string
  owner: string
  ssh_url: string
  https_url: string
  updated_at: string
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
  faces: {} as Record<string, string>,
  /** What each opened review says about itself, by number. */
  details: {} as Record<number, ReviewDetail>,
  /** The project `details` were read for, so a switch does not show another's. */
  detailsFor: null as string | null,
  /** The review a lookup is out for, so the panel can say it is coming. */
  loadingDetail: null as number | null,
  detailError: null as string | null,
  /** Everyone this project can hand a review to, once asked. */
  members: [] as Member[],
  /** The project `members` describes, so a switch does not show the last one. */
  membersFor: null as string | null,
  loadingMembers: false,
  membersError: null as string | null,
  /** The repositories the active profile's token can see, once asked. */
  repos: [] as ForgeRepo[],
  reposFor: null as string | null,
  loadingRepos: false,
  reposError: null as string | null
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
  /** How the forge itself writes a review's number: `!38` on GitLab, `#38` on GitHub. */
  const sigil = computed(() => (store.status?.kind === 'gitlab' ? '!' : '#'))
  /** What to call it out loud: "the forge" is what this app calls it, not what
      anybody who is about to click through to it calls it. */
  const forgeName = computed(() => {
    if (store.status?.kind === 'gitlab') return 'GitLab'
    if (store.status?.kind === 'github') return 'GitHub'
    return 'the forge'
  })

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
    // Asking who the token belongs to also tells the other side which commit
    // addresses that face answers for. Rows drawn before the answer arrived
    // have already been told there is no picture, so let them ask again.
    if (store.me?.avatar) forgetAvatars()
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

  /** The project a lookup belongs to: the account, plus the repository. */
  function projectId() {
    const slug = store.status?.slug
    if (!slug) return null
    return `${store.status?.kind}@${store.status?.host}/${slug.owner}/${slug.name}`
  }

  /**
   * Asks what one review says about itself, once per review.
   *
   * Everything here — who it is assigned to, what it is labelled, whether it
   * can be merged — is a second request the sidebar's list does not make, so
   * it waits until a review is actually being read. Kept afterwards because
   * clicking between two reviews is how they get compared.
   */
  async function loadReviewDetail(number: number, force = false) {
    const id = projectId()
    if (!usable.value || !id) return
    // A different project's numbers mean different reviews.
    if (store.detailsFor !== id) {
      store.details = {}
      store.detailsFor = id
    }
    if (!force && store.details[number]) return
    store.loadingDetail = number
    store.detailError = null
    try {
      store.details[number] = await invoke<ReviewDetail>('forge_review_detail', { number })
    } catch (error) {
      store.detailError = String(error)
    } finally {
      if (store.loadingDetail === number) store.loadingDetail = null
    }
  }

  /**
   * Asks who is on this project, once per project.
   *
   * Only the review dialog needs it, so it is asked for when that opens rather
   * than on every refresh: it is a request per project that most sessions
   * never make.
   */
  async function loadMembers(force = false) {
    const id = projectId()
    if (!usable.value || !id) {
      store.members = []
      return
    }
    if (!force && store.membersFor === id && store.members.length) return
    store.loadingMembers = true
    store.membersError = null
    try {
      store.members = await invoke<Member[]>('forge_members')
      store.membersFor = id
    } catch (error) {
      store.membersError = String(error)
      store.members = []
    } finally {
      store.loadingMembers = false
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

  /**
   * Asks the forge for every repository the token can see, once per account.
   *
   * Only the clone dialog needs it, so it is asked for when that opens. Unlike
   * the project-bound lookups this works with nothing open — the whole point
   * is choosing a repository before one exists locally.
   */
  async function loadRepos(force = false) {
    const id = profileId()
    if (!id || !store.status?.has_token) {
      store.repos = []
      return
    }
    if (!force && store.reposFor === id) return
    store.loadingRepos = true
    store.reposError = null
    try {
      store.repos = await invoke<ForgeRepo[]>('forge_repos')
      store.reposFor = id
    } catch (error) {
      store.reposError = String(error)
      store.repos = []
    } finally {
      store.loadingRepos = false
    }
  }

  return {
    store,
    usable,
    label,
    shortLabel,
    sigil,
    forgeName,
    refreshStatus,
    loadFaces,
    loadReviews,
    loadReviewDetail,
    loadMembers,
    loadMe,
    loadRepos,
    check,
    createReview: (draft: NewReview) => invoke<Review>('forge_create_review', { ...draft }),
    /** The forge's own new-review page, with the form already filled in. */
    compareUrl: (source: string, target: string, title: string, body: string) =>
      invoke<string>('forge_compare_url', { source, target, title, body }),
    open: (url: string) => invoke('open_external', { url }),
    /** The forge's token-creation page, with scopes and a name pre-filled. */
    tokenUrl: (kind: ForgeKind, host: string) =>
      invoke<string | null>('forge_token_url', { kind, host })
  }
}
