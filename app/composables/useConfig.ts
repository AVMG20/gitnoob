import { aimAt, invoke } from './useInvoke'
import { computed, reactive } from 'vue'

export type ForgeKind = 'none' | 'github' | 'gitlab'

export interface Project {
  path: string
  name: string
}

export interface Profile {
  id: string
  name: string
  forge: ForgeKind
  host: string
  git_name: string | null
  git_email: string | null
  /** Path to the private key this profile pushes and pulls with. */
  ssh_key: string | null
  /** `user.signingkey` — a path for ssh, a key id for gpg. */
  signing_key: string | null
  /** `gpg.format`: `openpgp`, `ssh` or `x509`. */
  signing_format: string | null
  /** `commit.gpgsign`. Null means the profile has no opinion and the
      repository's own configuration is left alone. */
  sign_commits: boolean | null
  /** `tag.gpgsign`. */
  sign_tags: boolean | null
  projects: Project[]
  /** Everything opened under this profile, newest first, tabs included. */
  recents: Project[]
  active_project: string | null
}

export type ReasoningLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high'

export interface AiSettings {
  model: string | null
  max_tokens: number
  reasoning: ReasoningLevel
  commit_style: 'plain' | 'conventional'
  /** What the model is told before it is shown a diff; null is the default. */
  commit_prompt: string | null
}

/** OpenRouter's effort levels, plus switching thinking off entirely. */
export const REASONING_LEVELS: { value: ReasoningLevel; label: string }[] = [
  { value: 'off', label: 'No thinking' },
  { value: 'minimal', label: 'Minimal' },
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' }
]

export interface GlobalSettings {
  ai: AiSettings
  graph_page_size: number
  auto_fetch_on_open: boolean
  auto_fetch_minutes: number
  auto_stash: boolean
  /** What checking out a remote branch does when its local branch has commits
      of its own while the remote also moved on. Merely behind is always a
      fast-forward, which is not a question. */
  diverged_checkout: 'ask' | 'rebase' | 'merge' | 'leave'
  show_avatars: boolean
  check_updates: boolean
  /** Check the signature on every commit the graph draws. */
  verify_signatures: boolean
}

export interface Config {
  version: number
  active_profile: string | null
  global: GlobalSettings
  profiles: Profile[]
}

export const OPENROUTER_SECRET = 'openrouter'

export type SettingsSection =
  | 'profiles'
  | 'ai'
  | 'appearance'
  | 'shortcuts'
  | 'behaviour'
  | 'updates'

const store = reactive({
  config: null as Config | null,
  /** Which secrets exist, keyed by keychain key. Never holds a value. */
  secrets: {} as Record<string, boolean>,
  settingsOpen: false,
  settingsSection: 'profiles' as SettingsSection,
  /** The project a click asked for, held only until the open lands. */
  opening: null as string | null
})

function apply(config: Config) {
  store.config = config
}

export function useConfig() {
  const config = computed(() => store.config)
  const profiles = computed(() => store.config?.profiles ?? [])
  const profile = computed(
    () => profiles.value.find((p) => p.id === store.config?.active_profile) ?? null
  )
  const projects = computed(() => profile.value?.projects ?? [])
  /** Everything opened under this profile, newest first. */
  const recents = computed(() => profile.value?.recents ?? [])
  const activeProject = computed(() => profile.value?.active_project ?? null)
  /**
   * Which tab to draw as the current one.
   *
   * The real answer lives in the config, and the config only says so once the
   * repository has been opened and the file re-read — two round trips after the
   * click. The reads are quick, but the strip sat on the old tab until they
   * landed, which reads as a slow app however fast the work behind it was. So
   * the tab a click asked for is drawn as current from the moment of the click,
   * and the config takes over when it catches up. A failed open clears the
   * intent and the highlight snaps back to whatever is really open.
   */
  const selectedProject = computed(() => store.opening ?? activeProject.value)
  const settings = computed(() => store.config?.global ?? null)

  async function load() {
    const loaded = await invoke<Config>('config_get').catch(() => null)
    if (loaded) apply(loaded)
    await refreshSecrets()
  }

  /** Records which secrets are present, so forms can say "set" without reading them. */
  async function refreshSecrets() {
    const keys = [OPENROUTER_SECRET]
    const forgeKey = await invoke<string | null>('forge_secret_key').catch(() => null)
    if (forgeKey) keys.push(forgeKey)
    for (const key of keys) {
      store.secrets[key] = await invoke<boolean>('secret_status', { key }).catch(() => false)
    }
  }

  const forgeSecretKey = () => invoke<string | null>('forge_secret_key')

  async function setSecret(key: string, value: string) {
    await invoke('secret_set', { key, value })
    store.secrets[key] = value.length > 0
  }

  return {
    store,
    config,
    profiles,
    profile,
    projects,
    recents,
    activeProject,
    selectedProject,
    settings,

    /** Draws a tab as current straight away, before its repository is open. */
    beginOpen(path: string) {
      store.opening = path
    },
    /** The open has landed, or failed: the config is the answer again. */
    endOpen() {
      store.opening = null
    },
    load,
    refreshSecrets,
    forgeSecretKey,
    setSecret,
    hasSecret: (key: string | null | undefined) => (key ? store.secrets[key] === true : false),

    openSettings(section: SettingsSection = 'profiles') {
      store.settingsSection = section
      store.settingsOpen = true
    },
    closeSettings() {
      store.settingsOpen = false
    },

    async saveGlobal(global: GlobalSettings) {
      apply(await invoke<Config>('config_set_global', { global }))
    },
    async saveProfile(next: Profile) {
      apply(await invoke<Config>('profile_save', { profile: next }))
      await refreshSecrets()
    },
    async deleteProfile(id: string) {
      apply(await invoke<Config>('profile_delete', { id }))
      await refreshSecrets()
    },
    /**
     * Switches profile, and stops addressing calls to the repository being left.
     *
     * The backend clears its own open path on a switch, but every call from the
     * window carries the repository it is about and the backend applies that
     * before running the command — so the next call re-opened the old
     * repository under the new profile's account. What came back was that
     * repository's remote read against the other forge: an empty sidebar, or a
     * 404 for a project that is not there. The window aims at nothing until
     * whatever the new profile has open has actually been opened.
     */
    async activateProfile(id: string) {
      aimAt(null)
      apply(await invoke<Config>('profile_activate', { id }))
      await refreshSecrets()
    },
    async closeProject(path: string) {
      apply(await invoke<Config>('project_close', { path }))
    },
    /** Takes one out of the recents. The folder on disk is not touched. */
    async forgetProject(path: string) {
      apply(await invoke<Config>('project_forget', { path }))
    },
    async reorderProjects(paths: string[]) {
      apply(await invoke<Config>('project_reorder', { paths }))
    },
    /** `open_repo` records the project itself, so just take the new config back. */
    async reload() {
      apply(await invoke<Config>('config_get'))
    },
    /** Null when there was nothing to change, so callers can stay quiet. */
    applyIdentity: () => invoke<string | null>('apply_identity')
  }
}

/** A blank profile for the "add profile" form. */
export function emptyProfile(): Profile {
  return {
    id: '',
    name: '',
    forge: 'none',
    host: '',
    git_name: null,
    git_email: null,
    ssh_key: null,
    signing_key: null,
    signing_format: null,
    sign_commits: null,
    sign_tags: null,
    projects: [],
    recents: [],
    active_project: null
  }
}

export const FORGE_LABELS: Record<ForgeKind, string> = {
  none: 'No forge',
  github: 'GitHub',
  gitlab: 'GitLab'
}

export const DEFAULT_HOSTS: Record<ForgeKind, string> = {
  none: '',
  github: 'github.com',
  gitlab: 'gitlab.com'
}
