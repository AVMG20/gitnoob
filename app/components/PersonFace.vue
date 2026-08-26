<script setup lang="ts">
import { computed } from 'vue'
import { initials, tint } from '~/composables/useAvatars'

/**
 * Somebody a review names, drawn as their picture or as the letters that
 * stand in for it.
 *
 * The graph's `Avatar` looks a face up by commit address and waits for the
 * answer; a forge hands the picture over with the person, so this one is
 * given what to draw and draws it. Every place a review names somebody uses
 * this, so a face looks the same in the header, the timeline and the sidebar.
 */
const props = withDefaults(
  defineProps<{
    login: string
    name?: string
    /** The picture as a `data:` URL, when the forge had one. */
    src?: string | null
    size?: number
    /** What the verdict badge says, for the faces that carry one. */
    badge?: 'approved' | 'changes_requested' | 'commented' | 'dismissed' | null
  }>(),
  { name: '', src: null, size: 20, badge: null }
)

const letters = computed(() => initials(props.name || props.login, props.login))
const style = computed(() => ({
  width: `${props.size}px`,
  height: `${props.size}px`,
  fontSize: `${Math.max(8, Math.round(props.size * 0.42))}px`,
  background: props.src ? 'transparent' : tint(props.login)
}))
</script>

<template>
  <span class="face" :style="style" :title="props.name || props.login" data-testid="face">
    <img v-if="props.src" :src="props.src" alt="" draggable="false" />
    <template v-else>{{ letters }}</template>
    <i v-if="props.badge" class="badge" :class="props.badge" />
  </span>
</template>

<style scoped>
.face {
  position: relative;
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  color: #fff;
  font-weight: 600;
  line-height: 1;
  user-select: none;
}

.face img {
  width: 100%;
  height: 100%;
  border-radius: 50%;
  object-fit: cover;
}

/* The verdict, as a dot on the shoulder of the face that gave it: a wall of
   reviewers reads as a row of ticks without a word being read. */
.badge {
  position: absolute;
  right: -2px;
  bottom: -2px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  border: 1.5px solid var(--bg-panel);
}

.badge.approved {
  background: var(--green);
}

.badge.changes_requested {
  background: var(--red);
}

.badge.commented,
.badge.dismissed {
  background: var(--text-faint);
}
</style>
