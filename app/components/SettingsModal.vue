<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import {
  Check,
  ExternalLink,
  Github,
  Gitlab,
  KeyRound,
  Plus,
  Settings2,
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
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'
import { useGit } from '~/composables/useGit'
import { describeKey, useSsh } from '~/composables/useSsh'

const config = useConfig()
const forge = useForge()
const ssh = useSsh()
const ai = useAi()
const git = useGit()

const section = computed(() => config.store.settingsSection)

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
 * Opens the forge's token page with the right scopes already selected.
 *
 * A true OAuth sign-in needs an application registered with each forge; this is
 * the same number of clicks without that dependency.
 */
async function signIn() {
  if (!editing.value || editing.value.forge === 'none') return
  const url = await forge.signinUrl(editing.value.forge, editing.value.host)
  if (!url) return
  await forge.open(url)
  git.note(`Opened ${editing.value.forge} — create the token, then paste it here`)
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

onMounted(async () => {
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
          :class="{ on: section === 'behaviour' }"
          @click="config.store.settingsSection = 'behaviour'"
        >
          <Settings2 :size="15" /> Behaviour
        </button>
        <p class="nav-note faint">
          Profiles hold their own forge, identity and open projects. Everything under AI and
          Behaviour is shared across all of them.
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
              <button class="btn btn-ghost signin" @click="signIn">
                <component :is="forgeIcon(editing.forge)" :size="14" />
                Sign in to {{ FORGE_LABELS[editing.forge] }}
                <ExternalLink :size="12" class="faint" />
              </button>
              <span class="hint faint">
                Opens the token page with the right scopes ticked. Create it, then paste it below.
              </span>
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
              These are applied to a repository only when you ask, from the profile menu — opening
              a repository never rewrites its config on its own.
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
  background: rgba(6, 9, 12, 0.66);
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
  box-shadow: 0 22px 60px rgba(0, 0, 0, 0.55);
}

.nav {
  background: #151a20;
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
  border-color: rgba(79, 156, 249, 0.5);
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
  color: #ef8d9c;
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

.signin {
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
  background: rgba(79, 156, 249, 0.12);
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
</style>
