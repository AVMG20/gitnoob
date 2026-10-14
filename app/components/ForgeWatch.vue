<script setup lang="ts">
import { onMounted, onUnmounted, watch } from 'vue'
import { useConfig } from '~/composables/useConfig'
import { useForge, type Review } from '~/composables/useForge'
import { useGit } from '~/composables/useGit'
import { changes, checkProfile, nextDelay, notify } from '~/composables/useForgeNudges'

/**
 * The forge, watched from the side of the window.
 *
 * Nothing to see here most of the time. It asks, when a repository opens,
 * whether it belongs to another profile; it keeps asking about checks still
 * running on your pull requests, less often the longer they run; it tells the
 * system when they finish or somebody reviews; and it holds the new-review
 * dialog for whoever asked for one away from the sidebar — a push, the home
 * tab.
 */
const forge = useForge()
const git = useGit()
const config = useConfig()

/** What the last reading of each project said, by project. */
const seen = new Map<string, Review[]>()

/** An hour of running checks is a pipeline nobody is waiting on any more. */
const GIVE_UP_MS = 60 * 60_000
/** Hidden this long, and the window is not being watched either. */
const HIDDEN_MS = 10 * 60_000

let timer: ReturnType<typeof setTimeout> | null = null
let delay: number | null = null
let pendingSince: number | null = null
let hiddenSince: number | null = null

const enabled = () => config.settings.value?.check_notifications !== false

/** Reviews whose checks and verdicts are worth following: yours, and the one
    for the branch checked out. */
function followed(list: Review[]) {
  const me = forge.store.me?.login ?? null
  return list.filter((one) => one.is_current || (!!me && one.author === me))
}

function stop() {
  if (timer) clearTimeout(timer)
  timer = null
}

/** Asks again later while something followed is still running. */
function schedule(list: Review[]) {
  stop()
  if (!enabled()) return
  const running = followed(list).some((one) => one.checks === 'pending')
  if (!running) {
    delay = null
    pendingSince = null
    return
  }
  pendingSince ??= Date.now()
  if (Date.now() - pendingSince > GIVE_UP_MS) return
  if (hiddenSince && Date.now() - hiddenSince > HIDDEN_MS) return
  delay = nextDelay(delay)
  timer = setTimeout(() => {
    timer = null
    if (!git.store.busy) void forge.loadReviews()
    else schedule(forge.store.reviews)
  }, delay)
}

watch(
  () => forge.store.reviews,
  (list) => {
    const key = forge.store.reviewsFor
    if (!key) return
    const before = seen.get(key) ?? null
    seen.set(
      key,
      list.map((one) => ({ ...one }))
    )
    if (enabled()) {
      const label = forge.shortLabel.value
      const sigil = forge.sigil.value
      for (const change of changes(before, list, forge.store.me?.login ?? null)) {
        void notify(`${change.text}: ${label} ${sigil}${change.number}`, change.title)
      }
    }
    schedule(list)
  }
)

// A repository just opened: is it the profile's?
watch(
  () => git.store.repo?.path,
  (path, before) => {
    if (path && path !== before) {
      delay = null
      pendingSince = null
      void checkProfile()
    }
  }
)

function visibility() {
  if (document.hidden) {
    hiddenSince ??= Date.now()
    return
  }
  const wasAway = hiddenSince && Date.now() - hiddenSince > HIDDEN_MS
  hiddenSince = null
  // Back after a long time away: one look now, and the schedule from there.
  if (wasAway && !timer && enabled()) {
    delay = null
    pendingSince = null
    if (forge.usable.value) void forge.loadReviews()
  }
}

onMounted(() => document.addEventListener('visibilitychange', visibility))
onUnmounted(() => {
  stop()
  document.removeEventListener('visibilitychange', visibility)
})

/** The dialog closes on its own; the request that opened it goes with it. */
function closeCompose() {
  forge.store.compose = null
}
</script>

<template>
  <ReviewDialog
    v-if="forge.store.compose && git.store.repo"
    @close="closeCompose"
  />
</template>
