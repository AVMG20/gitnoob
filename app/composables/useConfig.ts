import { invoke } from '@tauri-apps/api/core'
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
  projects: Project[]
  active_project: string | null
}

export type ReasoningLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high'

export interface AiSettings {
  model: string | null
  max_tokens: number
  reasoning: ReasoningLevel
  commit_style: 'plain' | 'conventional'
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
  show_avatars: boolean
}

export interface Config {
  version: number
  active_profile: string | null
  global: GlobalSettings
  profiles: Profile[]
}

export const OPENROUTER_SECRET = 'openrouter'

const store = reactive({
  config: null as Config | null,
  /** Which secrets exist, keyed by keychain key. Never holds a value. */
  secrets: {} as Record<string, boolean>,
  settingsOpen: false,
  settingsSection: 'profiles' as 'profiles' | 'ai' | 'behaviour'
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
  const activeProject = computed(() => profile.value?.active_project ?? null)
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
    activeProject,
    settings,
    load,
    refreshSecrets,
    forgeSecretKey,
    setSecret,
    hasSecret: (key: string | null | undefined) => (key ? store.secrets[key] === true : false),

    openSettings(section: 'profiles' | 'ai' | 'behaviour' = 'profiles') {
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
    async activateProfile(id: string) {
      apply(await invoke<Config>('profile_activate', { id }))
      await refreshSecrets()
    },
    async closeProject(path: string) {
      apply(await invoke<Config>('project_close', { path }))
    },
    async reorderProjects(paths: string[]) {
      apply(await invoke<Config>('project_reorder', { paths }))
    },
    /** `open_repo` records the project itself, so just take the new config back. */
    async reload() {
      apply(await invoke<Config>('config_get'))
    },
    applyIdentity: () => invoke<string>('apply_identity')
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
    projects: [],
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
