<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  Check,
  Download,
  ExternalLink,
  Github,
  Gitlab,
  Keyboard,
  KeyRound,
  Palette,
  Plus,
  RefreshCw,
  Settings2,
  ShieldCheck,
  Sparkles,
  Trash2,
  User,
  Users,
  X
} from 'lucide-vue-next'
import {
  DEFAULT_HOSTS,
  FORGE_LABELS,
  OPENROUTER_SECRET,
  REASONING_LEVELS,
  COMMIT_FLOW_DEFAULTS,
  emptyProfile,
  useConfig,
  type CommitFlowSettings,
  type ForgeKind,
  type Profile
} from '~/composables/useConfig'
import { forgetAvatars } from '~/composables/useAvatars'
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'
import { useGit } from '~/composables/useGit'
import { describeKey, useSsh } from '~/composables/useSsh'
import { SYSTEM_PAIR, useTheme } from '~/composables/useTheme'
import {
  SHORTCUTS,
  SHORTCUT_GROUPS,
  clashFor,
  eventChord,
  isRebindable,
  keyLabel,
  keysFor,
  type Shortcut
} from '~/composables/useShortcuts'
import { useColumns } from '~/composables/useColumns'
import { useUpdates } from '~/composables/useUpdates'
import { useZoom } from '~/composables/useZoom'
import { useSyntax } from '~/composables/useSyntax'
import { highlightWhole } from '~/composables/useHighlight'
import SearchSelect from '~/components/SearchSelect.vue'
import { useRewrite } from '~/composables/useRewrite'

const config = useConfig()
const forge = useForge()
const ssh = useSsh()
const ai = useAi()
const git = useGit()

// Remembering conflict resolutions is per repository, read from the backend
// for the one that is open: the list it is kept in names repositories by the
// path the backend knows them by.
const rewrite = useRewrite()
const rerere = ref(false)
async function readRerere() {
  if (git.store.repo) rerere.value = await rewrite.rerereEnabled()
}
async function toggleRerere(enabled: boolean) {
  const now = await rewrite.setRerere(enabled)
  if (now !== null) rerere.value = now
  // The global settings are saved whole, so the copy here has to carry the
  // new list or the next change elsewhere would put the old one back.
  await config.load()
}
onMounted(readRerere)
const { choice, themes, setTheme, contrast, contrasts, setContrast } = useTheme()

/** Porcelain and Graphite, for the card that follows the system between them. */
const systemSwatch = computed(() => [
  themes.find((one) => one.id === SYSTEM_PAIR.light)!.swatch,
  themes.find((one) => one.id === SYSTEM_PAIR.dark)!.swatch
])

/** How many themes are in a group, so the sentence above cannot go stale. */
const themeCount = (kind: string) => themes.filter((one) => one.kind === kind).length
const cols = useColumns()
const { zoom, steps: zoomSteps, setZoom } = useZoom()
const { syntax, choices, setSyntax } = useSyntax()

/** The picker hands back an id; the store is what decides to keep it. */
const chosenSyntax = computed({
  get: () => syntax.value,
  set: (id: string) => setSyntax(id)
})

/**
 * A sample coloured in the chosen theme, so the choice is made by looking.
 *
 * A single-file component on purpose: the template, the TypeScript inside
 * `<script>` and the comment above it are three different grammars, which is
 * exactly what the old highlighter could not tell apart and the clearest way to
 * see that a theme is doing its job.
 */
const PREVIEW = `<template>
  <p :class="tone">{{ label }}</p>
</template>

<script setup lang="ts">
// A comment, a string and a number.
const label = ref<string>('gitnoob')
const tone = compute(label, 3)
<\/script>`

const previewLines = computed(() => highlightWhole(PREVIEW, 'vue'))
const updates = useUpdates()

const section = computed(() => config.store.settingsSection)

// --- updates

/** Where to read what changed, before deciding to install it. */
const RELEASES_URL = 'https://github.com/AVMG20/gitnoob/releases'

/** The AppImage caveat is only worth saying on the platform it applies to. */
const linux = computed(() => navigator.userAgent.includes('Linux'))

/** What the install button says, which is mostly what it is doing. */
const installLabel = computed(() => {
  if (updates.store.stage === 'downloading') return 'Installing…'
  // The window is about to go; saying so is better than a button that looks
  // like it did nothing.
  if (updates.store.stage === 'ready') return 'Restarting…'
  return 'Download and install'
})

/** The button, as opposed to the quiet check at launch: this one reports. */
async function lookForUpdate() {
  await updates.checkForUpdate()
}

/** The keyboard, grouped the way the list is written, with empty groups gone. */
const shortcutGroups = computed(() =>
  SHORTCUT_GROUPS.map((group) => ({
    group,
    rows: SHORTCUTS.filter((one) => one.group === group)
  })).filter((one) => one.rows.length)
)

// --- moving a key
//
// A row's button listens for the next chord pressed on it. Escape gives up; a
// chord without ⌘/Ctrl or Alt is refused, since a bare letter would fire
// while reading; and one that another shortcut already answers to in the same
// place is refused with its name, rather than one of the two going quiet.
const capturing = ref<string | null>(null)
const captureNote = ref<string | null>(null)

function startCapture(id: string) {
  capturing.value = capturing.value === id ? null : id
  captureNote.value = null
}

function stopCapture() {
  capturing.value = null
}

const movedKeys = computed(() => config.settings.value?.shortcuts ?? {})

async function saveKeys(next: Record<string, string>) {
  await patchGlobal({ shortcuts: next })
}

async function onCaptureKey(event: KeyboardEvent, row: Shortcut) {
  if (capturing.value !== row.id) return
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Escape') {
    stopCapture()
    return
  }
  if (['Meta', 'Control', 'Shift', 'Alt'].includes(event.key)) return
  const chord = eventChord(event)
  if (!/^(mod|alt)\+/.test(chord)) {
    captureNote.value = `Hold ${keyLabel('mod')} or ${keyLabel('alt')} with the key.`
    return
  }
  const clash = clashFor(row, chord)
  if (clash) {
    captureNote.value = `${keyLabel(chord)} already does “${clash.label}”. Move that one first.`
    return
  }
  const next = { ...movedKeys.value }
  if (chord.toLowerCase() === row.keys.toLowerCase()) delete next[row.id]
  else next[row.id] = chord
  stopCapture()
  captureNote.value = null
  await saveKeys(next)
}

async function resetKey(id: string) {
  const next = { ...movedKeys.value }
  delete next[id]
  await saveKeys(next)
}

// --- the commit box
const commitFlow = computed<CommitFlowSettings>(() => ({
  ...COMMIT_FLOW_DEFAULTS,
  ...(config.settings.value?.commit ?? {})
}))

function patchCommit(patch: Partial<CommitFlowSettings>) {
  return patchGlobal({ commit: { ...commitFlow.value, ...patch } })
}

// --- profiles
const editing = ref<Profile | null>(null)
const token = ref('')
const tokenKey = ref<string | null>(null)
const saving = ref(false)

const forges: ForgeKind[] = ['none', 'github', 'gitlab']
const forgeIcon = (kind: ForgeKind) =>
  kind === 'github' ? Github : kind === 'gitlab' ? Gitlab : User

function edit(profile: Profile) {
  editing.value = JSON.parse(JSON.stringify(profile))
  token.value = ''
  ssh.clear()
}

function add() {
  editing.value = emptyProfile()
  token.value = ''
  ssh.clear()
}

/** Keeps the host in step with the forge unless the user typed their own. */
watch(
  () => editing.value?.forge,
  (kind, previous) => {
    if (!editing.value || !kind || previous === undefined) return
    const current = editing.value.host
    if (!current || current === DEFAULT_HOSTS[previous as ForgeKind]) {
      editing.value.host = DEFAULT_HOSTS[kind]
    }
  }
)

async function save() {
  if (!editing.value || !editing.value.name.trim()) return
  saving.value = true
  try {
    const isNew = !editing.value.id
    await config.saveProfile(editing.value)

    // A new profile gets its id from the backend; find it to store the token.
    const stored = config.profiles.value.find(
      (p) => p.id === editing.value?.id || (isNew && p.name === editing.value?.name)
    )
    if (token.value.trim() && stored) {
      await config.setSecret(`forge:${stored.id}`, token.value.trim())
      token.value = ''
    }
    await forge.refreshStatus()
    editing.value = null
  } finally {
    saving.value = false
  }
}

async function remove(profile: Profile) {
  if (config.profiles.value.length < 2) {
    git.note('Keep at least one profile', 'error')
    return
  }
  await config.deleteProfile(profile.id)
  if (editing.value?.id === profile.id) editing.value = null
}

/**
 * Opens the forge's token page with the scopes already ticked.
 *
 * The token is only made there, never here, so this leaves the field alone:
 * what comes back is pasted in by hand.
 */
async function openTokenPage() {
  if (!editing.value || editing.value.forge === 'none') return
  const url = await forge.tokenUrl(editing.value.forge, editing.value.host)
  if (!url) return
  await forge.open(url)
  git.note(`Opened ${FORGE_LABELS[editing.value.forge]} — create the token, then paste it here`)
}

async function testConnection() {
  const user = await forge.check()
  git.note(
    user ? `Connected as ${user}` : forge.store.error ?? 'Could not connect',
    user ? 'info' : 'error'
  )
}

// --- ssh keys
/**
 * Checks the key against the forge over ssh, which is a different question from
 * the token check above: the token is what the API accepts, the key is what
 * push and pull use.
 */
async function testSsh() {
  if (!editing.value) return
  const result = await ssh.test(editing.value.host, editing.value.ssh_key)
  const named = result.user ? `${result.message} (${result.user})` : result.message
  git.note(named, result.ok ? 'info' : 'error')
}

/** The picker writes a path; clearing it hands ssh back its own choice. */
function chooseKey(path: string) {
  if (!editing.value) return
  editing.value.ssh_key = path || null
  ssh.clear()
}

// --- AI
const apiKey = ref('')
const keySaved = computed(() => config.hasSecret(OPENROUTER_SECRET))

async function saveKey() {
  await config.setSecret(OPENROUTER_SECRET, apiKey.value.trim())
  apiKey.value = ''
  await ai.refreshStatus()
  git.note(keySaved.value ? 'OpenRouter key stored in the keychain' : 'OpenRouter key removed')
}

async function pickModel(id: string) {
  const settings = config.settings.value
  if (!settings) return
  await config.saveGlobal({ ...settings, ai: { ...settings.ai, model: id } })
  await ai.refreshStatus()
}

async function patchGlobal(patch: Record<string, unknown>) {
  const settings = config.settings.value
  if (!settings) return
  await config.saveGlobal({ ...settings, ...patch })
}

/**
 * The instructions the model writes commit messages under.
 *
 * Held here rather than bound straight at the config: it is typed into, and a
 * save on every keystroke would be a config write per letter. The box is
 * filled from the config when the settings open and saved when it loses focus.
 */
const commitPrompt = ref('')
const promptChanged = computed(
  () => commitPrompt.value.trim() !== (ai.store.status.default_commit_prompt ?? '').trim()
)

watch(
  () => [config.settings.value?.ai.commit_prompt, ai.store.status.default_commit_prompt] as const,
  ([stored, fallback]) => {
    // Only while the box is not being typed in, or a save landing under the
    // cursor would take the half-written sentence away.
    if (document.activeElement?.classList.contains('prompt')) return
    commitPrompt.value = stored ?? fallback ?? ''
  },
  { immediate: true }
)

async function saveCommitPrompt() {
  const text = commitPrompt.value.trim()
  const stored = config.settings.value?.ai.commit_prompt ?? null
  // An empty box means the default, and the default is stored as nothing at
  // all rather than as a copy that would go stale the moment it changed.
  const next = !text || text === (ai.store.status.default_commit_prompt ?? '').trim() ? null : text
  if (next === stored) return
  await patchAi({ commit_prompt: next })
}

async function resetCommitPrompt() {
  commitPrompt.value = ai.store.status.default_commit_prompt ?? ''
  if ((config.settings.value?.ai.commit_prompt ?? null) !== null) await patchAi({ commit_prompt: null })
}

/** The cap, kept to a range a request can actually be made with. */
async function saveMaxTokens(event: Event) {
  const input = event.target as HTMLInputElement
  const stored = config.settings.value?.ai.max_tokens ?? 0
  const typed = Number(input.value)
  const next = Number.isFinite(typed) && typed > 0 ? Math.min(200_000, Math.max(256, Math.round(typed))) : stored
  input.value = String(next)
  if (next !== stored) await patchAi({ max_tokens: next })
}

async function patchAi(patch: Record<string, unknown>) {
  const settings = config.settings.value
  if (!settings) return
  await config.saveGlobal({ ...settings, ai: { ...settings.ai, ...patch } })
  await ai.refreshStatus()
}

/** Esc closes it, the way it closes every other window in the app. */
function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') config.closeSettings()
}

onUnmounted(() => {
  window.removeEventListener('keydown', onKey)
  // Esc closes the window without the box ever losing focus, and a blur that
  // never fires is an edit that never lands. Saving here catches it.
  void saveCommitPrompt()
})

onMounted(async () => {
  window.addEventListener('keydown', onKey)
  await updates.version()
  tokenKey.value = await config.forgeSecretKey()
  await config.refreshSecrets()
  await ssh.loadKeys()
})
</script>

<template>
  <div class="scrim" @click.self="config.closeSettings()">
    <div class="panel">
      <nav class="nav">
        <div class="nav-title">Settings</div>
        <button
          class="nav-item"
          :class="{ on: section === 'profiles' }"
          @click="config.store.settingsSection = 'profiles'"
        >
          <Users :size="15" /> Profiles
        </button>
        <button
          class="nav-item"
          :class="{ on: section === 'ai' }"
          @click="config.store.settingsSection = 'ai'"
        >
          <Sparkles :size="15" /> AI
        </button>
        <button
          class="nav-item"
          :class="{ on: section === 'appearance' }"
          @click="config.store.settingsSection = 'appearance'"
        >
          <Palette :size="15" /> Appearance
        </button>
        <button
          class="nav-item"
          :class="{ on: section === 'shortcuts' }"
          @click="config.store.settingsSection = 'shortcuts'"
        >
          <Keyboard :size="15" /> Shortcuts
        </button>
        <button
          class="nav-item"
          :class="{ on: section === 'behaviour' }"
          @click="config.store.settingsSection = 'behaviour'"
        >
          <Settings2 :size="15" /> Behaviour
        </button>
        <button
          class="nav-item"
          :class="{ on: section === 'updates' }"
          @click="config.store.settingsSection = 'updates'"
        >
          <Download :size="15" /> Updates
          <span v-if="updates.store.stage === 'available'" class="nav-dot" />
        </button>
        <p class="nav-note faint">
          Each profile has its own forge, identity and projects. All other settings are shared.
        </p>
      </nav>

      <div class="content">
        <button class="close" title="Close" @click="config.closeSettings()"><X :size="16" /></button>

        <!-- Profiles -->
        <section v-if="section === 'profiles'">
          <h2>Profiles</h2>
          <p class="dim intro">
            One per account, such as work and personal. Switching changes the forge, the commit
            identity and the open tabs.
          </p>

          <div class="list">
            <div
              v-for="profile in config.profiles.value"
              :key="profile.id"
              class="entry"
              :class="{ active: profile.id === config.config.value?.active_profile }"
            >
              <component :is="forgeIcon(profile.forge)" :size="16" class="entry-icon" />
              <div class="entry-body">
                <div class="entry-name">
                  {{ profile.name }}
                  <span v-if="profile.id === config.config.value?.active_profile" class="pill">
                    active
                  </span>
                </div>
                <div class="faint small">
                  {{ FORGE_LABELS[profile.forge] }}
                  <template v-if="profile.host">· {{ profile.host }}</template>
                  · {{ profile.projects.length }}
                  {{ profile.projects.length === 1 ? 'project' : 'projects' }}
                </div>
              </div>
              <button
                v-if="profile.id !== config.config.value?.active_profile"
                class="btn tiny"
                @click="config.activateProfile(profile.id)"
              >
                Use
              </button>
              <button class="btn tiny" @click="edit(profile)">Edit</button>
              <button class="btn tiny danger" title="Delete" @click="remove(profile)">
                <Trash2 :size="13" />
              </button>
            </div>
          </div>

          <button class="btn btn-ghost add" @click="add">
            <Plus :size="14" /> Add a profile
          </button>

          <!-- Editor -->
          <div v-if="editing" class="editor">
            <h3>{{ editing.id ? 'Edit profile' : 'New profile' }}</h3>

            <label class="field">
              <span class="label">Name</span>
              <input v-model="editing.name" type="text" placeholder="Work" />
            </label>

            <div class="field">
              <span class="label">Forge</span>
              <div class="choices">
                <button
                  v-for="kind in forges"
                  :key="kind"
                  class="choice"
                  :class="{ on: editing.forge === kind }"
                  @click="editing.forge = kind"
                >
                  <component :is="forgeIcon(kind)" :size="14" />
                  {{ FORGE_LABELS[kind] }}
                </button>
              </div>
            </div>

            <label v-if="editing.forge !== 'none'" class="field">
              <span class="label">Host</span>
              <input v-model="editing.host" type="text" :placeholder="DEFAULT_HOSTS[editing.forge]" />
              <span class="hint faint">
                Change this for a self-hosted GitLab or GitHub Enterprise.
              </span>
            </label>

            <div v-if="editing.forge !== 'none'" class="field">
              <span class="label">
                <KeyRound :size="12" /> Access token
              </span>
              <span class="hint faint">
                Lets gitnoob list and open pull requests on {{ FORGE_LABELS[editing.forge] }}.
                Pushing and pulling don't need it; they use the SSH key.
              </span>
              <button class="btn btn-ghost token-link" @click="openTokenPage">
                <component :is="forgeIcon(editing.forge)" :size="14" />
                Open {{ FORGE_LABELS[editing.forge] }}'s token page
                <ExternalLink :size="12" class="faint" />
              </button>
              <input
                v-model="token"
                type="password"
                autocomplete="off"
                :placeholder="
                  editing.id && config.hasSecret(`forge:${editing.id}`)
                    ? 'Stored in the keychain — type to replace'
                    : 'Paste a personal access token'
                "
              />
              <span class="hint faint">
                Stored in the system keychain, not in the config file.
              </span>
            </div>

            <div class="field">
              <span class="label">
                <KeyRound :size="12" /> SSH key
              </span>
              <select
                class="key-select"
                :value="editing.ssh_key ?? ''"
                @change="chooseKey(($event.target as HTMLSelectElement).value)"
              >
                <option value="">Let ssh choose (agent or ~/.ssh/config)</option>
                <option v-for="key in ssh.store.keys" :key="key.path" :value="key.path">
                  {{ describeKey(key) }}
                </option>
                <option
                  v-if="editing.ssh_key && !ssh.store.keys.some((k) => k.path === editing!.ssh_key)"
                  :value="editing.ssh_key"
                >
                  {{ editing.ssh_key }}
                </option>
              </select>
              <span class="hint faint">
                While this profile is active, fetch, pull and push use only this key. Useful when
                work and personal accounts share a machine.
              </span>
              <button
                v-if="editing.forge !== 'none'"
                class="btn btn-ghost signin"
                :disabled="ssh.store.testing"
                @click="testSsh"
              >
                <component :is="forgeIcon(editing.forge)" :size="14" />
                {{ ssh.store.testing ? 'Connecting…' : `Test ssh to ${editing.host || DEFAULT_HOSTS[editing.forge]}` }}
              </button>
              <p
                v-if="ssh.store.result"
                class="hint"
                :class="ssh.store.result.ok ? 'ok' : 'err'"
              >
                {{ ssh.store.result.message }}
              </p>
            </div>

            <!-- Signing sits beside the ssh key because it is the same idea
                 and, for ssh signing, usually the same file. Every field is
                 optional: what the profile says nothing about is left exactly
                 as this machine already had it. -->
            <div class="field">
              <span class="label">
                <ShieldCheck :size="12" /> Signing key
              </span>
              <div class="signing-row">
                <select
                  class="sign-format"
                  :value="editing.signing_format ?? ''"
                  @change="
                    editing.signing_format =
                      ($event.target as HTMLSelectElement).value || null
                  "
                >
                  <option value="">Leave as is</option>
                  <option value="ssh">SSH</option>
                  <option value="openpgp">GPG</option>
                  <option value="x509">X.509</option>
                </select>
                <input
                  type="text"
                  :value="editing.signing_key ?? ''"
                  :placeholder="
                    editing.signing_format === 'ssh'
                      ? '~/.ssh/id_ed25519.pub'
                      : 'Key id, or a path for ssh'
                  "
                  spellcheck="false"
                  @input="
                    editing.signing_key =
                      ($event.target as HTMLInputElement).value.trim() || null
                  "
                />
              </div>
              <span class="hint faint">
                Set as <span class="mono">user.signingkey</span> and
                <span class="mono">gpg.format</span> in repositories opened with this profile. Your
                global git config is not changed.
              </span>
            </div>

            <label class="check">
              <input
                type="checkbox"
                :checked="editing.sign_commits === true"
                @change="
                  editing.sign_commits = ($event.target as HTMLInputElement).checked ? true : null
                "
              />
              <span>
                Sign every commit
                <span class="sub faint">
                  Sets <span class="mono">commit.gpgsign</span> in repositories opened with this
                  profile. Unticked leaves each repository's setting alone.
                </span>
              </span>
            </label>

            <label class="check">
              <input
                type="checkbox"
                :checked="editing.sign_tags === true"
                @change="
                  editing.sign_tags = ($event.target as HTMLInputElement).checked ? true : null
                "
              />
              <span>
                Sign annotated tags
                <span class="sub faint">Sets <span class="mono">tag.gpgsign</span>.</span>
              </span>
            </label>

            <div class="two">
              <label class="field">
                <span class="label">Commit name</span>
                <input v-model="editing.git_name" type="text" placeholder="Robin Vale" />
              </label>
              <label class="field">
                <span class="label">Commit email</span>
                <input v-model="editing.git_email" type="text" placeholder="you@example.com" />
              </label>
            </div>
            <p class="hint faint no-top">
              Repositories opened with this profile commit with this name and email. You get a note
              when that changes a repository's settings.
            </p>

            <label v-if="editing.forge !== 'none'" class="field">
              <span class="label">Only for these owners (optional)</span>
              <input
                type="text"
                data-testid="profile-owners"
                placeholder="acme, acme-labs"
                :value="(editing.owners ?? []).join(', ')"
                @change="
                  editing.owners = ($event.target as HTMLInputElement).value
                    .split(',')
                    .map((one) => one.trim())
                    .filter(Boolean)
                "
              />
            </label>
            <p v-if="editing.forge !== 'none'" class="hint faint no-top">
              For when two profiles share a host: repositories under these accounts or groups use
              this profile.
            </p>

            <div class="editor-actions">
              <button class="btn btn-ghost" @click="editing = null">Cancel</button>
              <button
                v-if="editing.id && editing.forge !== 'none'"
                class="btn btn-ghost"
                :disabled="forge.store.checking"
                @click="testConnection"
              >
                {{ forge.store.checking ? 'Checking…' : 'Test connection' }}
              </button>
              <button
                class="btn btn-primary"
                :disabled="saving || !editing.name.trim()"
                @click="save"
              >
                <Check :size="14" /> Save profile
              </button>
            </div>
            <p v-if="forge.store.error" class="err">{{ forge.store.error }}</p>
          </div>
        </section>

        <!-- AI -->
        <section v-else-if="section === 'ai'">
          <h2>AI</h2>
          <p class="dim intro">
            Requests go through OpenRouter, so one key works for every model. The key is stored in
            the keychain and only sent to OpenRouter.
          </p>

          <label class="field">
            <span class="label"><KeyRound :size="12" /> OpenRouter API key</span>
            <span class="key-row">
              <input
                v-model="apiKey"
                type="password"
                autocomplete="off"
                :placeholder="keySaved ? 'Stored — type to replace' : 'sk-or-…'"
              />
              <button class="btn btn-primary" :disabled="!apiKey.trim()" @click="saveKey">
                Save
              </button>
              <button v-if="keySaved" class="btn btn-ghost" @click="((apiKey = ''), saveKey())">
                Remove
              </button>
            </span>
            <span class="hint" :class="keySaved ? 'ok' : 'faint'">
              {{ keySaved ? 'A key is stored in the keychain.' : 'No key stored yet.' }}
            </span>
          </label>

          <div class="field">
            <span class="label">Model</span>
            <ModelPicker :selected="config.settings.value?.ai.model ?? null" @pick="pickModel" />
          </div>

          <label class="field">
            <span class="label">Thinking</span>
            <select
              :value="config.settings.value?.ai.reasoning"
              @change="patchAi({ reasoning: ($event.target as HTMLSelectElement).value })"
            >
              <option v-for="level in REASONING_LEVELS" :key="level.value" :value="level.value">
                {{ level.label }}
              </option>
            </select>
          </label>
          <p class="hint faint no-top">
            How much the model reasons first. Thinking tokens cost money and commit messages rarely
            need them. Models that can't reason ignore this.
          </p>

          <label class="field narrow">
            <span class="label">Max tokens per answer</span>
            <input
              type="number"
              class="max-tokens"
              min="256"
              max="200000"
              step="100"
              :value="config.settings.value?.ai.max_tokens"
              @change="saveMaxTokens($event)"
            />
          </label>
          <p class="hint faint no-top">
            The longest answer the model may give, thinking included. Raise it if answers get cut
            off; lower it to limit cost. Large conflicts get more.
          </p>

          <!-- The instructions themselves rather than a choice between two of
               them: what makes a good commit message here is a house rule, and
               a dropdown can only ever hold somebody else's. -->
          <label class="field">
            <span class="label">Commit message instructions</span>
            <textarea
              v-model="commitPrompt"
              class="prompt"
              rows="12"
              spellcheck="false"
              placeholder="What the model is told before it is shown the diff"
              @blur="saveCommitPrompt"
            />
          </label>
          <p class="hint faint no-top">
            Sent to the model along with the diff. Saved when you click away.
            <button v-if="promptChanged" class="link" @click="resetCommitPrompt">
              Put the default back
            </button>
          </p>

          <p class="hint faint">
            With a key and a model set, the commit box gets a Generate button and the conflict
            resolver gets "Resolve with AI".
          </p>
        </section>

        <!-- Appearance -->
        <section v-else-if="section === 'appearance'">
          <h2>Appearance</h2>
          <p class="dim intro">
            {{ themes.length }} themes: {{ themeCount('Light') }} light,
            {{ themeCount('Semi-dark') }} semi-dark, {{ themeCount('Dark') }} dark.
          </p>

          <div class="themes">
            <!-- The default: Porcelain by day and Graphite by night, following
                 whatever the system is set to. -->
            <button
              class="theme"
              :class="{ on: choice === 'system' }"
              @click="setTheme('system')"
            >
              <span class="swatch split">
                <span
                  v-for="(one, at) in systemSwatch"
                  :key="at"
                  class="half"
                  :style="{ background: one[0] }"
                >
                  <span class="chip" :style="{ background: one[1] }"></span>
                </span>
              </span>
              <span class="theme-name">Match system</span>
              <span class="faint small">Porcelain or Graphite</span>
              <Check v-if="choice === 'system'" :size="13" class="tick" />
            </button>
            <button
              v-for="one in themes"
              :key="one.id"
              class="theme"
              :class="{ on: one.id === choice }"
              @click="setTheme(one.id)"
            >
              <span class="swatch" :style="{ background: one.swatch[0] }">
                <span class="chip" :style="{ background: one.swatch[1] }"></span>
                <span class="chip text" :style="{ background: one.swatch[2] }"></span>
              </span>
              <span class="theme-name">{{ one.name }}</span>
              <span class="faint small">{{ one.kind }}</span>
              <Check v-if="one.id === choice" :size="13" class="tick" />
            </button>
          </div>

          <h3 class="sub">Contrast</h3>
          <p class="dim intro">
            How strong dimmed text and panel borders are.
          </p>
          <div class="sizes">
            <button
              v-for="one in contrasts"
              :key="one.id"
              class="size"
              :class="{ on: one.id === contrast }"
              @click="setContrast(one.id)"
            >
              {{ one.name }}
              <span class="faint small block">{{ one.note }}</span>
            </button>
          </div>

          <h3 class="sub">Text size</h3>
          <p class="dim intro">
            Scales text, rows and the graph together. {{ keyLabel('mod+=') }} and
            {{ keyLabel('mod+-') }} step through these sizes, {{ keyLabel('mod+0') }} resets.
          </p>
          <div class="sizes">
            <button
              v-for="factor in zoomSteps"
              :key="factor"
              class="size"
              :class="{ on: Math.abs(factor - zoom) < 0.001 }"
              @click="setZoom(factor)"
            >
              {{ Math.round(factor * 100) }}%
              <span v-if="factor === 1" class="faint small block">standard</span>
            </button>
          </div>

          <h3 class="sub">Syntax colours</h3>
          <p class="dim intro">
            Colours for code in diffs and open files. These are the same themes VS Code uses, each
            marked dark or light.
          </p>
          <SearchSelect
            v-model="chosenSyntax"
            :options="choices"
            placeholder="Choose a theme…"
            empty="No theme by that name."
          />
          <pre class="preview"><code
            v-for="(line, at) in previewLines"
            :key="at"
            class="preview-line"
            v-html="line || '&nbsp;'"
          /></pre>

          <h3 class="sub">Columns in the commit list</h3>
          <p class="dim intro">
            Drag the edge between two headings to resize a column; double-click it to reset.
            Right-clicking the headings shows this list too.
          </p>
          <div class="cols">
            <label v-for="column in cols.columns" :key="column.id" class="check">
              <input type="checkbox" :checked="cols.state.shown[column.id]"
                     @change="cols.toggle(column.id)" />
              <span>{{ column.label }}</span>
            </label>
          </div>
          <button class="btn tiny" @click="cols.resetWidths()">Reset the widths</button>
        </section>

        <!-- Shortcuts -->
        <section v-else-if="section === 'shortcuts'">
          <h2>Shortcuts</h2>
          <p class="dim intro">
            Every shortcut and where it works. None of them work while a dialog is open or while
            you're typing in a text field.
          </p>

          <div v-for="one in shortcutGroups" :key="one.group" class="keys-group">
            <h3>{{ one.group }}</h3>
            <div v-for="row in one.rows" :key="row.id" class="keys-row">
              <kbd class="keys" :class="{ moved: movedKeys[row.id] }">{{
                keyLabel(keysFor(row.id))
              }}</kbd>
              <span class="keys-what">
                {{ row.label }}
                <span v-if="row.note" class="faint small block">{{ row.note }}</span>
                <span v-if="capturing === row.id && captureNote" class="small block capture-note">
                  {{ captureNote }}
                </span>
              </span>
              <span class="faint small keys-where">
                {{ row.where }}
                <span v-if="isRebindable(row)" class="rebind">
                  <button
                    class="btn tiny"
                    :class="{ on: capturing === row.id }"
                    :title="capturing === row.id ? 'Press the new keys, or Esc' : 'Bind this to other keys'"
                    @click="startCapture(row.id)"
                    @keydown="onCaptureKey($event, row)"
                    @blur="capturing === row.id && stopCapture()"
                  >
                    {{ capturing === row.id ? 'Press keys…' : 'Change' }}
                  </button>
                  <button
                    v-if="movedKeys[row.id]"
                    class="btn tiny ghost"
                    :title="`Back to ${keyLabel(row.keys)}`"
                    @click="resetKey(row.id)"
                  >
                    Reset
                  </button>
                </span>
              </span>
            </div>
          </div>
        </section>

        <!-- Updates -->
        <section v-else-if="section === 'updates'">
          <h2>Updates</h2>
          <p class="dim intro">
            Releases for macOS, Windows and Linux are published on GitHub. The app can download and
            install them itself.
          </p>

          <div class="field">
            <span class="label">Installed version</span>
            <span class="version-row">
              <strong class="version">{{ updates.store.current || '—' }}</strong>
              <button class="btn btn-ghost" :disabled="updates.busy.value" @click="lookForUpdate">
                <RefreshCw :size="13" :class="{ spin: updates.store.stage === 'checking' }" />
                Check for updates
              </button>
            </span>
          </div>

          <!-- Nothing on offer, and we looked. -->
          <p v-if="updates.store.stage === 'none'" class="hint ok">
            This is the newest release.
          </p>

          <p v-else-if="updates.store.stage === 'error'" class="hint bad">
            {{ updates.store.error }}
          </p>

          <div
            v-else-if="updates.store.stage !== 'idle' && updates.store.stage !== 'checking'"
            class="offer"
          >
            <div class="offer-head">
              <strong>Version {{ updates.store.version }} is available</strong>
              <span v-if="updates.store.date" class="faint">released {{ updates.store.date }}</span>
            </div>

            <pre v-if="updates.store.notes" class="notes">{{ updates.store.notes }}</pre>

            <!-- The bar only appears once there is something to measure; a
                 server that sends no length would otherwise sit at zero. -->
            <div v-if="updates.store.stage === 'downloading'" class="progress">
              <div class="track">
                <div class="bar" :style="{ width: `${updates.progress.value}%` }" />
              </div>
              <span class="faint">
                {{ updates.store.total ? `${updates.progress.value}%` : 'Downloading…' }}
              </span>
            </div>

            <div class="offer-actions">
              <button
                class="btn btn-primary"
                :disabled="updates.busy.value || updates.store.stage === 'ready'"
                @click="updates.install()"
              >
                <Download :size="13" />
                {{ installLabel }}
              </button>
              <button
                class="btn btn-ghost"
                :disabled="updates.busy.value"
                @click="updates.dismiss()"
              >
                Not now
              </button>
            </div>
            <p class="hint faint">
              The app closes while it updates, then reopens. Your repositories are not touched.
            </p>
          </div>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.check_updates"
              @change="patchGlobal({ check_updates: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Look for a new version at launch</strong>
              <span class="faint block">
                Asks GitHub for the latest release when the app opens. When off, only the button
                above checks.
              </span>
            </span>
          </label>

          <p class="hint faint">
            Updates are signed with the project's release key and verified before they are
            installed.
            <a href="#" @click.prevent="forge.open(RELEASES_URL)">
              All releases <ExternalLink :size="11" />
            </a>
          </p>
          <p v-if="linux" class="hint faint no-top">
            On Linux this works for the AppImage. If you installed the .deb or .rpm, update through
            your package manager.
          </p>
        </section>

        <!-- Behaviour -->
        <section v-else>
          <h2>Behaviour</h2>
          <p class="dim intro">What the app does for you around git.</p>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.auto_fetch_on_open"
              @change="patchGlobal({ auto_fetch_on_open: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Fetch when a project opens</strong>
              <span class="faint block">
                Keeps the ahead/behind counts up to date from the start.
              </span>
            </span>
          </label>

          <label class="field narrow">
            <span class="label">Keep fetching every</span>
            <span class="inline">
              <input
                type="number"
                min="0"
                max="120"
                :value="config.settings.value?.auto_fetch_minutes"
                @change="patchGlobal({ auto_fetch_minutes: Number(($event.target as HTMLInputElement).value) })"
              />
              <span class="faint">minutes (0 turns it off)</span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.auto_stash"
              @change="patchGlobal({ auto_stash: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Stash and restore around branch switches and pulls</strong>
              <span class="faint block">
                Uncommitted changes are stashed first and restored afterwards. Without this, git
                refuses while you have changes.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.update_refs ?? false"
              @change="patchGlobal({ update_refs: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Move stacked branches along with a rebase</strong>
              <span class="faint block">
                Branches built on the rebased branch move with it (git's
                <span class="mono">--update-refs</span>, git 2.38+). Branches at the tip, such as
                backups, and branches that track a remote branch with a different name are left
                alone.
              </span>
            </span>
          </label>

          <label v-if="git.store.repo" class="check" data-test="rerere-setting">
            <input
              type="checkbox"
              :checked="rerere"
              @change="toggleRerere(($event.target as HTMLInputElement).checked)"
            />
            <span>
              <strong>Remember how conflicts were resolved in this repository</strong>
              <span class="faint block">
                Uses git's rerere to suggest resolutions you made before; you still accept each one.
                Once used, git does this in the terminal too for this repository, unless
                <span class="mono">rerere.enabled</span> is false in your git config.
              </span>
            </span>
          </label>

          <label class="field">
            <span class="label">When a checked-out remote branch has diverged locally</span>
            <select
              :value="config.settings.value?.diverged_checkout"
              @change="patchGlobal({ diverged_checkout: ($event.target as HTMLSelectElement).value })"
            >
              <option value="ask">Ask what to do</option>
              <option value="rebase">Rebase my commits onto the remote</option>
              <option value="merge">Merge the remote into my branch</option>
              <option value="leave">Just switch, and leave them diverged</option>
            </select>
          </label>
          <p class="hint faint no-top">
            Double-clicking a remote branch checks out its local branch and fast-forwards it. This
            is for when both sides have new commits.
          </p>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.worktree_replaces_tab"
              @change="
                patchGlobal({ worktree_replaces_tab: ($event.target as HTMLInputElement).checked })
              "
            />
            <span>
              <strong>Open worktrees in the current tab</strong>
              <span class="faint block">
                Clicking a worktree in the sidebar opens it in this tab instead of a new one.
                Right-click for the other option.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              data-testid="setting-post-merge"
              :checked="config.settings.value?.post_merge_switch !== false"
              @change="
                patchGlobal({ post_merge_switch: ($event.target as HTMLInputElement).checked })
              "
            />
            <span>
              <strong>After merging a review, switch to the main branch</strong>
              <span class="faint block">
                If the merged branch is checked out and there are no uncommitted changes, switches
                to main and fast-forwards it. Deleting the branch is offered, never automatic.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.offer_review_after_push !== false"
              @change="
                patchGlobal({ offer_review_after_push: ($event.target as HTMLInputElement).checked
                })
              "
            />
            <span>
              <strong>Offer to open a pull request after a branch's first push</strong>
              <span class="faint block">
                Shows a notice with the pull request filled in from the branch's commits. Nothing is
                created until you confirm.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.check_notifications !== false"
              @change="
                patchGlobal({ check_notifications: ($event.target as HTMLInputElement).checked })
              "
            />
            <span>
              <strong>Watch checks and reviews on my pull requests</strong>
              <span class="faint block">
                Sends a system notification when checks pass or fail, or when someone reviews.
              </span>
            </span>
          </label>

          <label class="field">
            <span class="label">When a repository belongs to another profile</span>
            <select
              :value="config.settings.value?.profile_match ?? 'ask'"
              @change="patchGlobal({ profile_match: ($event.target as HTMLSelectElement).value as 'ask' })"
            >
              <option value="ask">Ask whether to use that profile for it</option>
              <option value="switch">Use that profile for it</option>
              <option value="off">Do nothing</option>
            </select>
          </label>
          <p class="hint faint no-top">
            A repository on another forge, or under an owner another profile is for. The other
            profile's token, SSH key and identity are used for that repository only. Its remote URL
            is never changed without asking.
          </p>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.verify_signatures"
              @change="
                patchGlobal({ verify_signatures: ($event.target as HTMLInputElement).checked })
              "
            />
            <span>
              <strong>Check signatures in the commit list</strong>
              <span class="faint block">
                Marks signed commits and flags ones that can't be trusted. Off by default because it
                runs gpg or ssh-keygen for every visible commit, which is slow on large
                repositories. The selected commit is always checked.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.show_avatars"
              @change="
                patchGlobal({ show_avatars: ($event.target as HTMLInputElement).checked });
                forgetAvatars()
              "
            />
            <span>
              <strong>Show a picture for each author</strong>
              <span class="faint block">
                Looked up by email on GitHub and Gravatar (and GitLab for GitLab projects), then
                cached. When off, initials are shown and nothing is sent.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="commitFlow.ticket_prefix"
              @change="patchCommit({ ticket_prefix: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Start the commit message with the branch's ticket</strong>
              <span class="faint block">
                On a branch like <span class="mono">feature/ABC-123-login</span> an empty message
                starts with <span class="mono">ABC-123: </span>. The AI always sees the branch name.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="commitFlow.guard"
              @change="patchCommit({ guard: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Look over what is staged before committing</strong>
              <span class="faint block">
                Warns about conflict markers, real-looking keys and tokens, and files like
                <span class="mono">.env</span> or <span class="mono">id_rsa</span>. You can still
                commit.
              </span>
            </span>
          </label>

          <label class="check" :class="{ dim: !commitFlow.guard }">
            <input
              type="checkbox"
              :disabled="!commitFlow.guard"
              :checked="commitFlow.guard_debug"
              @change="patchCommit({ guard_debug: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Also point out debugging left in</strong>
              <span class="faint block">
                <span class="mono">console.log</span>, <span class="mono">dbg!</span>,
                <span class="mono">dd(</span> and <span class="mono">var_dump</span> on added
                lines. Off by default, since plenty of code uses them on purpose.
              </span>
            </span>
          </label>

          <label class="field narrow">
            <span class="label">Commits loaded per page</span>
            <input
              type="number"
              min="100"
              max="5000"
              step="100"
              :value="config.settings.value?.graph_page_size"
              @change="patchGlobal({ graph_page_size: Number(($event.target as HTMLInputElement).value) })"
            />
          </label>

          <h3 class="sub">Staying in sync with the remote</h3>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.keep_trunk_current !== false"
              @change="
                patchGlobal({ keep_trunk_current: ($event.target as HTMLInputElement).checked })
              "
            />
            <span>
              <strong>Keep the local main branch current</strong>
              <span class="faint block">
                After each fetch, fast-forwards main to the remote when it has no commits of its own
                and isn't checked out. Nothing can be lost.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.announce_remote_moves !== false"
              @change="
                patchGlobal({ announce_remote_moves: ($event.target as HTMLInputElement).checked })
              "
            />
            <span>
              <strong>Say when the remote moves on</strong>
              <span class="faint block">
                When a background fetch brings new commits for your branch or main, a notice says
                how many and offers to pull them.
              </span>
            </span>
          </label>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.suggest_tidy !== false"
              @change="patchGlobal({ suggest_tidy: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Offer to tidy away finished branches</strong>
              <span class="faint block">
                After a fetch, the sidebar lists local branches that were merged, squashed, or whose
                remote branch was deleted. Nothing is deleted without a click, and deletions can be
                undone.
              </span>
            </span>
          </label>

          <label class="field">
            <span class="label">Update from main</span>
            <select
              :value="config.settings.value?.update_from_trunk ?? 'auto'"
              @change="patchGlobal({ update_from_trunk: ($event.target as HTMLSelectElement).value })"
            >
              <option value="auto">Rebase branches only I have, merge into pushed ones</option>
              <option value="rebase">Always rebase onto main</option>
              <option value="merge">Always merge main in</option>
            </select>
          </label>
          <p class="hint faint no-top">
            Rebasing keeps history linear, but a pushed branch then needs a force push. Merging
            never rewrites pushed commits.
          </p>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.warn_push_to_trunk"
              @change="
                patchGlobal({ warn_push_to_trunk: ($event.target as HTMLInputElement).checked })
              "
            />
            <span>
              <strong>Ask before pushing straight to main</strong>
              <span class="faint block">
                For teams where everything reaches main through a pull request.
              </span>
            </span>
          </label>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: grid;
  place-items: center;
  background: var(--overlay);
  animation: fade-in 0.1s ease-out;
}

/* A preferences window more than a dialog: the sections down the left in the
   chrome's tone, the page itself on the right. */
.panel {
  display: grid;
  grid-template-columns: 200px minmax(0, 1fr);
  width: 880px;
  max-width: calc(100vw - 40px);
  height: 660px;
  max-height: calc(100vh - 60px);
  background: var(--bg);
  border-radius: 10px;
  overflow: hidden;
  box-shadow: var(--shadow-pop);
  animation: rise-in 0.1s ease-out;
}

@keyframes fade-in {
  from {
    opacity: 0;
  }
}

@keyframes rise-in {
  from {
    opacity: 0;
    transform: scale(0.985);
  }
}

.nav {
  background: var(--canvas);
  border-right: 1px solid var(--line);
  padding: 14px 8px;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.nav-title {
  padding: 0 8px 10px;
  font-size: 13px;
  font-weight: 650;
  color: var(--text-dim);
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 28px;
  padding: 3px 8px;
  border-radius: var(--radius);
  color: var(--text);
  font-weight: 500;
  text-align: left;
}

.nav-item svg {
  color: var(--text-faint);
}

.nav-item:hover {
  background: var(--bg-hover);
}

.nav-item.on {
  background: var(--bg-active);
  font-weight: 600;
}

.nav-item.on svg {
  color: var(--accent);
}

.nav-note {
  margin: auto 0 0;
  padding: 8px;
  font-size: 11px;
  line-height: 1.5;
}

.content {
  position: relative;
  overflow-y: auto;
  padding: 20px 26px 28px;
}

.close {
  position: absolute;
  right: 10px;
  top: 10px;
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  color: var(--text-faint);
  border-radius: var(--radius);
}

.close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

h2 {
  margin: 0 0 4px;
  font-size: 17px;
  font-weight: 650;
}

h3 {
  margin: 0 0 10px;
  font-size: 13px;
  font-weight: 600;
}

.intro {
  margin: 0 0 16px;
  font-size: 12.5px;
  max-width: 64ch;
  line-height: 1.5;
}

/* One bordered list, its rows divided by hairlines inside it. */
.list {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--line-soft);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.themes {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: 8px;
}

/* A theme: a picture of it, its name, and a ring in the accent round the one
   you have. */
.theme {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 1px;
  padding: 6px 6px 8px;
  background: var(--bg);
  border: 1px solid var(--line-soft);
  border-radius: var(--radius-lg);
  text-align: left;
  transition: border-color 0.1s;
}

.theme:hover {
  border-color: var(--line);
}

.theme.on,
.theme.on:hover {
  border-color: var(--primary);
  box-shadow: 0 0 0 1px var(--primary);
}

.theme > .faint.small,
.theme > .theme-name {
  padding: 0 3px;
}

.swatch {
  display: flex;
  align-items: center;
  gap: 5px;
  width: 100%;
  height: 40px;
  margin-bottom: 5px;
  padding: 0 8px;
  border-radius: var(--radius);
  box-shadow: inset 0 0 0 1px var(--border-soft);
}

/* Half the Porcelain card and half the Graphite one. */
.swatch.split {
  padding: 0;
  gap: 0;
  overflow: hidden;
}

.half {
  flex: 1;
  display: flex;
  align-items: center;
  height: 100%;
  padding: 0 7px;
}

.chip {
  width: 18px;
  height: 8px;
  border-radius: 2px;
}

.chip.text {
  flex: none;
  margin-left: auto;
}

.theme-name {
  font-weight: 600;
}

.tick {
  position: absolute;
  right: 11px;
  top: 11px;
  padding: 2px;
  box-sizing: content-box;
  border-radius: var(--radius-pill);
  color: var(--primary-fg);
  background: var(--primary);
}

.entry {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  background: var(--bg);
}

.entry + .entry {
  border-top: 1px solid var(--line-soft);
}

.entry.active .entry-icon {
  color: var(--accent);
  opacity: 1;
}

.entry-icon {
  flex: none;
  opacity: 0.8;
}

.entry-body {
  flex: 1;
  min-width: 0;
}

.entry-name {
  display: flex;
  align-items: center;
  gap: 7px;
  font-weight: 600;
}

.small {
  font-size: 11px;
}

.tiny {
  min-height: 24px;
  font-size: 12px;
  padding: 1px 9px;
  border-radius: var(--radius-sm);
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--line);
}

.tiny.danger {
  color: var(--red-soft);
}

.add {
  margin-top: 10px;
}

.editor {
  margin-top: 16px;
  padding: 16px;
  border-radius: var(--radius-lg);
  background: var(--canvas);
  border: 1px solid var(--line-soft);
}

.field {
  display: block;
  margin-bottom: 14px;
}

.field.narrow input {
  width: 120px;
}

.label {
  display: flex;
  align-items: center;
  gap: 5px;
  margin-bottom: 5px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-dim);
}

.field input[type='text'],
.field input[type='password'],
.field select,
.field textarea {
  width: 100%;
}

.field input[type='text'],
.field input[type='password'] {
  height: 30px;
  padding: 4px 9px;
}

/* Instructions to a model are prose, but prose with its line breaks meant, so
   it is read in the same monospace the commit box uses. */
.prompt {
  display: block;
  font-family: var(--mono);
  font-size: 11.5px;
  line-height: 1.6;
}

/* A sentence that does something, inside a sentence that does not. */
.link {
  padding: 0;
  color: var(--text);
  font-weight: 500;
  color: var(--accent-soft);
  text-decoration: underline;
  text-underline-offset: 2px;
}

select {
  height: 30px;
  padding: 4px 26px 4px 9px;
  color: var(--text);
  background-color: var(--bg);
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
}


.token-link {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  justify-content: center;
  padding: 7px;
  margin-bottom: 7px;
}

.key-select {
  margin-bottom: 2px;
}

.key-row {
  display: flex;
  gap: 7px;
}

.key-row input {
  flex: 1;
}

.two {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}

.inline {
  display: flex;
  align-items: center;
  gap: 8px;
}

.hint {
  display: block;
  margin-top: 6px;
  font-size: 11.5px;
  line-height: 1.5;
}

.hint.no-top {
  margin-top: -6px;
  margin-bottom: 14px;
}

.ok {
  color: var(--green);
}

.err {
  margin: 10px 0 0;
  font-size: 12px;
  color: var(--red);
}

/* A segmented control: one bordered strip, the chosen segment raised in it. */
.choices {
  display: inline-flex;
  gap: 2px;
  padding: 2px;
  border-radius: var(--radius);
  background: var(--canvas);
  border: 1px solid var(--line);
}

.choice {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 24px;
  padding: 2px 12px;
  border-radius: var(--radius-sm);
  color: var(--text-dim);
  font-size: 12.5px;
  font-weight: 500;
}

.choice:hover {
  color: var(--text);
}

.choice.on,
.choice.on:hover {
  color: var(--text);
  background: var(--bg);
  box-shadow: 0 0 0 1px var(--line), 0 1px 2px var(--shadow);
  font-weight: 600;
}

.editor-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.check {
  display: flex;
  gap: 9px;
  margin-bottom: 12px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  max-width: 66ch;
}

.check .block,
.check .sub {
  font-weight: 400;
}

.check input {
  margin-top: 2px;
}

.block {
  display: block;
  margin-top: 3px;
  font-size: 11.5px;
  line-height: 1.5;
}

/* The keyboard page. Three columns — the key, what it does, where it works —
   so a row is read across rather than as a sentence to parse. */
.keys-group {
  margin-bottom: 18px;
}

/* A group is one rounded list, its rows divided by hairlines inside it. */
.keys-group {
  padding: 0 12px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line-soft);
}

.keys-group h3 {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-dim);
  margin: 8px 0 4px;
}

.keys-row {
  display: grid;
  grid-template-columns: 96px 1fr 190px;
  gap: 12px;
  align-items: baseline;
  padding: 6px 0;
  border-top: 1px solid var(--line-soft);
  font-size: 12.5px;
}

/* Written as a key cap: a box with a heavier bottom edge. */
.keys {
  font-family: inherit;
  font-size: 11px;
  font-weight: 600;
  text-align: center;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  background: var(--bg);
  border: 1px solid var(--line);
  border-bottom-width: 2px;
  color: var(--text);
  white-space: nowrap;
}

.keys-what {
  min-width: 0;
}

.keys-where {
  text-align: right;
}

/* A key moved from the one it ships with: the accent, so a changed binding is
   findable at a glance. */
.keys.moved {
  border-color: var(--primary-line);
}

.rebind {
  display: inline-flex;
  gap: 4px;
  margin-left: 6px;
}

.rebind .btn.on {
  background: var(--primary-bg);
  border-color: var(--primary-line);
}

.capture-note {
  color: var(--amber-soft);
  margin-top: 2px;
}

.sub {
  font-size: 13px;
  font-weight: 600;
  margin: 24px 0 4px;
}

/* The four column names sit in a row: they are one choice, not four settings
   stacked down the page. */
.cols {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  margin-bottom: 10px;
}

/* The scheme cards carry three bars — keyword, string, comment — which is what
   tells two schemes apart at a glance and what the eye compares between them. */
/* The sample sits on the window's own background rather than the theme's, which
   is where a diff shows it too: the row tint carries which side a line is on,
   so only the token colours come from the syntax theme. */
.preview {
  margin: 10px 0 18px;
  padding: 8px 10px;
  overflow-x: auto;
  background: var(--bg);
  border: 1px solid var(--line-soft);
  border-radius: var(--radius);
}

.preview-line {
  display: block;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre;
  color: var(--text);
}

/* The sizes read as one row of steps rather than as a list, so which way is
   bigger is the direction the eye already travels. */
.sizes {
  display: inline-flex;
  flex-wrap: wrap;
  gap: 2px;
  padding: 2px;
  margin-bottom: 18px;
  border-radius: var(--radius);
  background: var(--canvas);
  border: 1px solid var(--line);
}

/* A segmented control: the chosen step raised out of the strip. */
.size {
  min-width: 64px;
  padding: 4px 12px;
  font-size: 12.5px;
  font-weight: 500;
  line-height: 1.3;
  text-align: center;
  color: var(--text-dim);
  border-radius: var(--radius-sm);
}

.size:hover {
  color: var(--text);
}

.size.on,
.size.on:hover {
  color: var(--text);
  background: var(--bg);
  box-shadow: 0 0 0 1px var(--line), 0 1px 2px var(--shadow);
  font-weight: 600;
}

.cols .check {
  margin-bottom: 0;
}

/* --- updates */

/* The one place in the nav that ever has news, so it says so quietly rather
   than opening a dialog over whatever you were doing. */
.nav-dot {
  width: 6px;
  height: 6px;
  margin-left: auto;
  border-radius: 50%;
  background: var(--danger);
}

.version-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.version {
  font-family: var(--mono);
  font-size: 13px;
}

.bad {
  color: var(--red);
}

.spin {
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.offer {
  margin: 12px 0 18px;
  padding: 12px 14px;
  border-radius: var(--radius-lg);
  background: var(--canvas);
  border: 1px solid var(--line-soft);
}

.offer-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  font-size: 13px;
}

/* Release notes as they were written — a list of changes reads as a list even
   without a markdown renderer, and pre keeps the line breaks that make it one.
   Tall ones scroll here rather than pushing the buttons off the panel. */
.notes {
  max-height: 200px;
  margin: 10px 0 0;
  overflow-y: auto;
  font-family: var(--mono);
  font-size: 11px;
  line-height: 1.6;
  white-space: pre-wrap;
  color: var(--text-dim);
}

.progress {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 12px;
  font-size: 11px;
}

.progress .track {
  flex: 1;
  min-width: 0;
  height: 4px;
  border-radius: var(--radius-pill);
  background: var(--bg-raised);
}

.progress .bar {
  width: 0;
  height: 100%;
  border-radius: var(--radius-pill);
  background: var(--accent);
  transition: width 0.2s linear;
}

.offer-actions {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}

/* Format and key on one line: the format decides what the key field means, so
   reading them apart from each other is reading half a setting. */
.signing-row {
  display: flex;
  gap: 8px;
}

.sign-format {
  flex: none;
  width: 140px;
}

.signing-row input {
  flex: 1;
  min-width: 0;
}

.check .sub {
  display: block;
  font-size: 11px;
}
</style>