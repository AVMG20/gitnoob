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
  Users
} from 'lucide-vue-next'
import {
  DEFAULT_HOSTS,
  FORGE_LABELS,
  OPENROUTER_SECRET,
  REASONING_LEVELS,
  emptyProfile,
  useConfig,
  type ForgeKind,
  type Profile
} from '~/composables/useConfig'
import { forgetAvatars } from '~/composables/useAvatars'
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'
import { useGit } from '~/composables/useGit'
import { describeKey, useSsh } from '~/composables/useSsh'
import { useTheme } from '~/composables/useTheme'
import { SHORTCUTS, SHORTCUT_GROUPS, keyLabel } from '~/composables/useShortcuts'
import { useColumns } from '~/composables/useColumns'
import { useUpdates } from '~/composables/useUpdates'
import { useZoom } from '~/composables/useZoom'
import { useSyntax } from '~/composables/useSyntax'

const config = useConfig()
const forge = useForge()
const ssh = useSsh()
const ai = useAi()
const git = useGit()
const { theme, themes, setTheme, contrast, contrasts, setContrast } = useTheme()

/** How many themes are in a group, so the sentence above cannot go stale. */
const themeCount = (kind: string) => themes.filter((one) => one.kind === kind).length
const cols = useColumns()
const { zoom, steps: zoomSteps, setZoom } = useZoom()
const { syntax, schemes, setSyntax } = useSyntax()
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

onUnmounted(() => window.removeEventListener('keydown', onKey))

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
          Profiles hold their own forge, identity and open projects. Everything under AI,
          Appearance and Behaviour is shared across all of them.
        </p>
      </nav>

      <div class="content">
        <button class="close" @click="config.closeSettings()">✕</button>

        <!-- Profiles -->
        <section v-if="section === 'profiles'">
          <h2>Profiles</h2>
          <p class="dim intro">
            One per context: work on GitLab, personal on GitHub. Switching swaps the forge, the
            commit identity and the open project tabs.
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
                A password gitnoob uses to talk to {{ FORGE_LABELS[editing.forge] }} on your
                behalf, so it can list pull requests and open them for you. Make one under
                Settings → Developer settings → Personal access tokens, and paste it here.
                Pushing and pulling do not need it — that is the ssh key below.
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
                Kept in the operating system's keychain, never in the config file.
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
                Pins this profile to one key, so a work account and a personal account can share a
                machine without ssh offering the wrong one first. Every fetch, pull and push made
                while this profile is active uses it and nothing else.
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
                Written to each repository as <span class="mono">user.signingkey</span> and
                <span class="mono">gpg.format</span> when it is opened under this profile. Your
                global git config is never touched.
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
                  Sets <span class="mono">commit.gpgsign</span> on repositories opened under this
                  profile. Unticked leaves whatever each repository already says.
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
                <input v-model="editing.git_name" type="text" placeholder="Arno Visker" />
              </label>
              <label class="field">
                <span class="label">Commit email</span>
                <input v-model="editing.git_email" type="text" placeholder="you@example.com" />
              </label>
            </div>
            <p class="hint faint no-top">
              Every repository you open under this profile commits as this person. Opening one
              says so at the bottom of the window when it changes what was there before.
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
            Everything goes through OpenRouter, so one key covers every model. The key is stored in
            the keychain and only ever leaves this machine in a request to OpenRouter.
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

          <div class="two">
            <label class="field">
              <span class="label">Commit message style</span>
              <select
                :value="config.settings.value?.ai.commit_style"
                @change="patchAi({ commit_style: ($event.target as HTMLSelectElement).value })"
              >
                <option value="plain">Plain — imperative summary, short why</option>
                <option value="conventional">Conventional Commits</option>
              </select>
            </label>
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
          </div>
          <p class="hint faint no-top">
            OpenRouter's own effort levels, passed on to whichever model you picked. Thinking
            tokens are billed, and a commit message rarely needs them — a model that cannot reason
            ignores this either way.
          </p>

          <p class="hint faint">
            With a key and a model set, you get a Generate button on the commit box and
            "Resolve with AI" in the conflict resolver.
          </p>
        </section>

        <!-- Appearance -->
        <section v-else-if="section === 'appearance'">
          <h2>Appearance</h2>
          <p class="dim intro">
            {{ themes.length }} themes, counted here rather than claimed: {{ themeCount('Light') }}
            light, {{ themeCount('Semi-dark') }} semi-dark, {{ themeCount('Dark') }} dark. The
            choice is shared across every repository.
          </p>

          <div class="themes">
            <button
              v-for="one in themes"
              :key="one.id"
              class="theme"
              :class="{ on: one.id === theme }"
              @click="setTheme(one.id)"
            >
              <span class="swatch" :style="{ background: one.swatch[0] }">
                <span class="chip" :style="{ background: one.swatch[1] }"></span>
                <span class="chip text" :style="{ background: one.swatch[2] }"></span>
              </span>
              <span class="theme-name">{{ one.name }}</span>
              <span class="faint small">{{ one.kind }}</span>
              <Check v-if="one.id === theme" :size="13" class="tick" />
            </button>
          </div>

          <h3 class="sub">Contrast</h3>
          <p class="dim intro">
            How hard the dimmed text and the lines between panels work, on whichever theme is on.
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
            How large the window draws everything — text, rows and the graph together, so the
            commit list stays in step with the lines drawn through it.
            {{ keyLabel('mod+=') }} and {{ keyLabel('mod+-') }} step through the same sizes, and
            {{ keyLabel('mod+0') }} comes back here.
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
            Which scheme code is coloured with in a diff and in an open file. Each has a light
            variant of its own, so the choice holds whichever theme is on above.
          </p>
          <div class="schemes">
            <button
              v-for="one in schemes"
              :key="one.id"
              class="scheme"
              :class="{ on: one.id === syntax }"
              @click="setSyntax(one.id)"
            >
              <span class="scheme-swatch">
                <span v-for="(colour, at) in one.swatch" :key="at" :style="{ background: colour }" />
              </span>
              <span class="scheme-name">{{ one.name }}</span>
              <span class="faint small">{{ one.from }}</span>
              <Check v-if="one.id === syntax" :size="13" class="tick" />
            </button>
          </div>

          <h3 class="sub">Columns in the commit list</h3>
          <p class="dim intro">
            Which columns are drawn. Drag the line between two headings to resize one, and
            double-click that line to put it back. Right-clicking the headings offers the same
            list.
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
            Every key the window listens for, and where it has to be pressed. None of them fire
            while a dialog is open, or while the caret is in a box that takes text.
          </p>

          <div v-for="one in shortcutGroups" :key="one.group" class="keys-group">
            <h3>{{ one.group }}</h3>
            <div v-for="row in one.rows" :key="row.id" class="keys-row">
              <kbd class="keys">{{ keyLabel(row.keys) }}</kbd>
              <span class="keys-what">
                {{ row.label }}
                <span v-if="row.note" class="faint small block">{{ row.note }}</span>
              </span>
              <span class="faint small keys-where">{{ row.where }}</span>
            </div>
          </div>
        </section>

        <!-- Updates -->
        <section v-else-if="section === 'updates'">
          <h2>Updates</h2>
          <p class="dim intro">
            Releases are built for macOS, Windows and Linux and published on GitHub. This window
            can fetch one and install it over itself.
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
              The app closes while the new version is written, and comes back on its own. Nothing
              in your repositories is touched.
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
                One request to GitHub when the window opens, asking only which release is newest.
                Off, and the button above is the only check.
              </span>
            </span>
          </label>

          <p class="hint faint">
            Every download is signed with the project's release key and verified before it is
            written, so a file that key never signed is refused.
            <a href="#" @click.prevent="forge.open(RELEASES_URL)">
              All releases <ExternalLink :size="11" />
            </a>
          </p>
          <p v-if="linux" class="hint faint no-top">
            On Linux this works for the AppImage. Installed from the .deb or .rpm, update through
            your package manager or download the next release by hand.
          </p>
        </section>

        <!-- Behaviour -->
        <section v-else>
          <h2>Behaviour</h2>
          <p class="dim intro">The quiet housekeeping that saves remembering commands.</p>

          <label class="check">
            <input
              type="checkbox"
              :checked="config.settings.value?.auto_fetch_on_open"
              @change="patchGlobal({ auto_fetch_on_open: ($event.target as HTMLInputElement).checked })"
            />
            <span>
              <strong>Fetch when a project opens</strong>
              <span class="faint block">
                So the ahead/behind counts on screen are true straight away rather than whatever
                they were last session.
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
              <span class="faint">minutes — 0 turns it off</span>
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
                Uncommitted work is stashed, the operation runs, then the work comes back. Without
                this, git refuses and you tidy up by hand.
              </span>
            </span>
          </label>

          <label class="field">
            <span class="label">If a remote branch's local branch has diverged when checking it out</span>
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
            Double-clicking a remote branch checks out its local branch and pulls it up to date
            when that is a plain fast-forward. This decides what happens on the rare day both
            sides have commits of their own.
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
                Puts a mark beside every signed commit, and says which of them cannot be trusted.
                Off by default because it runs gpg or ssh-keygen once per commit on the page,
                which on a large repository is the slowest thing on the screen. The commit you
                have selected is checked either way.
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
                Looked up from the author's email address on GitHub and Gravatar — and on GitLab
                for a GitLab project — then kept on this machine. Off, and initials are drawn
                instead and nothing leaves the app.
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
}

.panel {
  display: grid;
  grid-template-columns: 208px minmax(0, 1fr);
  width: 860px;
  max-width: calc(100vw - 40px);
  height: 640px;
  max-height: calc(100vh - 60px);
  background: var(--bg-panel);
  border: 1px solid var(--line);
  border-radius: 11px;
  overflow: hidden;
  box-shadow: 0 22px 60px var(--shadow-strong);
}

.nav {
  background: var(--bg-deep);
  border-right: 1px solid var(--line);
  padding: 12px 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.nav-title {
  padding: 0 10px 10px;
  font-size: 15px;
  font-weight: 600;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 7px 10px;
  border-radius: 6px;
  color: var(--text-dim);
  text-align: left;
}

.nav-item:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.nav-item.on {
  background: var(--bg-active);
  color: var(--text);
  font-weight: 600;
}

.nav-note {
  margin: auto 0 0;
  padding: 10px;
  font-size: 11px;
  line-height: 1.5;
}

.content {
  position: relative;
  overflow-y: auto;
  padding: 18px 22px 26px;
}

.close {
  position: absolute;
  right: 12px;
  top: 12px;
  color: var(--text-faint);
  padding: 4px 7px;
  border-radius: 5px;
}

.close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

h2 {
  margin: 0 0 4px;
  font-size: 17px;
}

h3 {
  margin: 0 0 12px;
  font-size: 13.5px;
}

.intro {
  margin: 0 0 18px;
  font-size: 12.5px;
  max-width: 62ch;
  line-height: 1.55;
}

.list {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.themes {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 10px;
}

.theme {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  padding: 9px 10px;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 8px;
  text-align: left;
}

.theme:hover {
  background: var(--bg-hover);
}

.theme.on {
  border-color: var(--accent);
}

.swatch {
  display: flex;
  align-items: center;
  gap: 5px;
  width: 100%;
  height: 34px;
  padding: 0 7px;
  border-radius: 5px;
  border: 1px solid var(--line);
}

.chip {
  width: 16px;
  height: 9px;
  border-radius: 3px;
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
  right: 8px;
  top: 8px;
  color: var(--accent);
}

.entry {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 9px 11px;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 8px;
}

.entry.active {
  border-color: color-mix(in srgb, var(--accent) 50%, transparent);
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
  font-size: 11.5px;
  padding: 3px 8px;
  border: 1px solid var(--line);
}

.tiny.danger {
  color: var(--red-soft);
}

.add {
  margin-top: 10px;
}

.editor {
  margin-top: 20px;
  padding: 15px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: var(--bg-raised);
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
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.field input[type='text'],
.field input[type='password'],
.field select {
  width: 100%;
}

select {
  padding: 6px 8px;
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 5px;
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
  margin-top: 5px;
  font-size: 11px;
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

.choices {
  display: flex;
  gap: 6px;
}

.choice {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border: 1px solid var(--line);
  border-radius: 6px;
  color: var(--text-dim);
  font-size: 12.5px;
}

.choice:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.choice.on {
  border-color: var(--accent);
  color: var(--text);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.editor-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.check {
  display: flex;
  gap: 10px;
  margin-bottom: 16px;
  cursor: pointer;
  font-size: 12.5px;
  max-width: 66ch;
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

.keys-group h3 {
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
  margin: 0 0 6px;
}

.keys-row {
  display: grid;
  grid-template-columns: 96px 1fr 190px;
  gap: 12px;
  align-items: baseline;
  padding: 5px 0;
  border-top: 1px solid var(--line);
  font-size: 12.5px;
}

.keys {
  font-family: inherit;
  font-size: 11.5px;
  font-weight: 600;
  text-align: center;
  padding: 2px 6px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--bg-raised);
  color: var(--text);
  white-space: nowrap;
}

.keys-what {
  min-width: 0;
}

.keys-where {
  text-align: right;
}

.sub {
  font-size: 13px;
  margin: 22px 0 4px;
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
.schemes {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
  gap: 8px;
  margin-bottom: 18px;
}

.scheme {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 9px 10px;
  text-align: left;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 7px;
}

.scheme:hover {
  border-color: var(--text-faint);
}

.scheme.on {
  border-color: var(--accent);
  background: var(--bg-active);
}

.scheme-swatch {
  display: flex;
  gap: 3px;
  margin-bottom: 3px;
}

.scheme-swatch span {
  width: 22px;
  height: 6px;
  border-radius: 2px;
}

.scheme-name {
  font-size: 12px;
  color: var(--text);
}

/* The sizes read as one row of steps rather than as a list, so which way is
   bigger is the direction the eye already travels. */
.sizes {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 18px;
}

.size {
  min-width: 62px;
  padding: 6px 10px;
  font-size: 12px;
  text-align: center;
  color: var(--text-dim);
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 6px;
}

.size:hover {
  color: var(--text);
  border-color: var(--text-faint);
}

.size.on {
  color: var(--text);
  background: var(--bg-active);
  border-color: var(--accent);
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
  background: var(--accent);
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
  border: 1px solid color-mix(in srgb, var(--accent) 50%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--accent) 10%, transparent);
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
  border-radius: 2px;
  background: var(--bg-hover);
}

.progress .bar {
  width: 0;
  height: 100%;
  border-radius: 2px;
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