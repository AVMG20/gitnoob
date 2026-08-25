<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  Check,
  ChevronDown,
  Download,
  Github,
  Gitlab,
  Sparkles,
  User,
  UserCog
} from 'lucide-vue-next'
import { FORGE_LABELS, useConfig } from '~/composables/useConfig'
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'
import { useUpdates } from '~/composables/useUpdates'

const config = useConfig()
const forge = useForge()
const ai = useAi()
const updates = useUpdates()

const open = ref(false)
const profile = computed(() => config.profile.value)

const forgeIcon = computed(() => {
  if (profile.value?.forge === 'github') return Github
  if (profile.value?.forge === 'gitlab') return Gitlab
  return User
})

/** The signed-in user's own picture, when the forge knows one. */
const face = computed(() => forge.store.me?.avatar ?? null)

/** The picture for one profile in the switcher, or null to fall back to a logo. */
function faceFor(id: string) {
  return forge.store.faces[id] ?? null
}

function iconFor(forgeKind: string) {
  if (forgeKind === 'github') return Github
  if (forgeKind === 'gitlab') return Gitlab
  return User
}

// Asked for when the menu opens rather than on load: nothing else uses the
// other profiles' faces, and the answer is cached for the rest of the run.
watch(open, (showing) => {
  if (showing) forge.loadFaces()
})

async function pick(id: string) {
  open.value = false
  if (id === profile.value?.id) return
  await config.activateProfile(id)
  await forge.refreshStatus()
}
</script>

<template>
  <div class="wrap">
    <button class="pill-btn" :class="{ on: open }" @click="open = !open">
      <!-- The face when the forge knows one, its logo when it does not: either
           way the button says which account this is, not merely that there is
           one. -->
      <img v-if="face" class="face" :src="face" alt="" draggable="false" />
      <component :is="forgeIcon" v-else :size="14" />
      <span class="who">{{ profile?.name ?? 'No profile' }}</span>
      <ChevronDown :size="13" class="faint" />
    </button>

    <template v-if="open">
      <div class="scrim" @click="open = false" />
      <div class="menu">
        <div class="section-title">Current profile</div>
        <div class="current">
          <img v-if="face" class="face big" :src="face" alt="" draggable="false" />
          <component :is="forgeIcon" v-else :size="16" />
          <div class="grow">
            <div class="strong">{{ profile?.name }}</div>
            <div class="faint small">
              {{ FORGE_LABELS[profile?.forge ?? 'none'] }}
              <template v-if="profile?.host">· {{ profile.host }}</template>
            </div>
          </div>
        </div>

        <div class="rows">
          <!-- Named for what it is from the outside: an account you are signed
               in to, not the token that does the signing in. -->
          <div v-if="profile?.forge && profile.forge !== 'none'" class="row">
            <span class="faint">{{ FORGE_LABELS[profile.forge] }} account</span>
            <span v-if="forge.store.me" class="mono small">
              {{ forge.store.me.login }}
            </span>
            <span v-else-if="forge.store.status?.has_token" class="ok">signed in</span>
            <span v-else class="faint">not connected</span>
          </div>
          <div class="row">
            <span class="faint">Commit identity</span>
            <span v-if="profile?.git_name" class="mono small">
              {{ profile.git_name }}
            </span>
            <span v-else class="faint">not set</span>
          </div>
          <div class="row">
            <span class="faint">AI model</span>
            <span v-if="ai.store.status.model" class="mono small truncate">
              {{ ai.store.status.model }}
            </span>
            <span v-else class="faint">not configured</span>
          </div>
        </div>

        <div class="divider" />
        <div class="section-title">Switch profile</div>
        <button
          v-for="candidate in config.profiles.value"
          :key="candidate.id"
          class="item"
          @click="pick(candidate.id)"
        >
          <Check v-if="candidate.id === profile?.id" :size="14" class="tick" />
          <span v-else class="tick-space" />
          <img
            v-if="faceFor(candidate.id)"
            class="face"
            :src="faceFor(candidate.id)!"
            alt=""
            draggable="false"
          />
          <span v-else class="face placeholder">
            <component :is="iconFor(candidate.forge)" :size="11" />
          </span>
          <span class="grow">{{ candidate.name }}</span>
          <span class="faint small">{{ FORGE_LABELS[candidate.forge] }}</span>
        </button>

        <div class="divider" />
        <button class="item" @click="((open = false), config.openSettings('profiles'))">
          <UserCog :size="14" /> Manage profiles
        </button>
        <button class="item" @click="((open = false), config.openSettings('ai'))">
          <Sparkles :size="14" /> AI settings
        </button>
        <!-- Only when there is one. The check at launch is quiet, so this is
             where a new version turns up without going looking for it. -->
        <button
          v-if="updates.store.stage === 'available'"
          class="item"
          @click="((open = false), config.openSettings('updates'))"
        >
          <Download :size="14" /> Update to {{ updates.store.version }}
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.wrap {
  position: relative;
}

.pill-btn {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 5px 9px;
  border-radius: 6px;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  color: var(--text);
  max-width: 200px;
}

.pill-btn:hover,
.pill-btn.on {
  background: var(--bg-active);
}

.face {
  width: 17px;
  height: 17px;
  border-radius: 50%;
  object-fit: cover;
  flex: none;
}

.face.big {
  width: 22px;
  height: 22px;
}

/* A profile whose forge has no picture still gets a round slot, so the names
   in the switcher line up whether or not there is a face beside them. */
.face.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-active);
  color: var(--text-dim);
}

.who {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

.scrim {
  position: fixed;
  inset: 0;
  z-index: 40;
}

.menu {
  position: absolute;
  right: 0;
  top: calc(100% + 6px);
  z-index: 41;
  width: 320px;
  padding: 4px;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 9px;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.55);
}

.current {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  margin: 0 2px 6px;
  border-radius: 7px;
  background: var(--bg-panel);
}

.grow {
  flex: 1;
  min-width: 0;
}

.strong {
  font-weight: 600;
}

.small {
  font-size: 11px;
}

.rows {
  padding: 2px 12px 8px;
}

.row {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  padding: 2px 0;
  font-size: 11.5px;
}

.ok {
  color: var(--green);
}

.item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 6px 10px;
  border-radius: 6px;
  text-align: left;
  font-size: 12.5px;
}

.item:hover {
  background: var(--bg-active);
}

.tick {
  color: var(--green);
  flex: none;
}

.tick-space {
  width: 14px;
  flex: none;
}

.divider {
  height: 1px;
  margin: 5px 6px;
  background: var(--line);
}
</style>
