<script setup lang="ts">
import { computed } from 'vue'
import { avatarFor, initials, tint } from '~/composables/useAvatars'

const props = withDefaults(
  defineProps<{
    name: string
    email: string
    size?: number
  }>(),
  { size: 18 }
)

const src = computed(() => avatarFor(props.email))
const letters = computed(() => initials(props.name, props.email))
</script>

<template>
  <span
    class="avatar"
    :style="{
      width: `${props.size}px`,
      height: `${props.size}px`,
      fontSize: `${Math.round(props.size * 0.42)}px`,
      background: src ? 'transparent' : tint(props.email)
    }"
    :title="props.email ? `${props.name} <${props.email}>` : props.name"
  >
    <!-- While the lookup is out there is neither a picture nor a reason to draw
         initials that are about to be replaced; the circle stays empty for the
         moment it takes. -->
    <img v-if="src" :src="src" alt="" draggable="false" />
    <template v-else-if="src === null">{{ letters }}</template>
  </span>
</template>

<style scoped>
.avatar {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  overflow: hidden;
  color: #fff;
  font-weight: 600;
  letter-spacing: 0.02em;
  user-select: none;
  line-height: 1;
}

.avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
</style>
