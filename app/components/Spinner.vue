<script setup lang="ts">
defineProps<{ size?: number }>()
</script>

<template>
  <span class="spin" :style="{ width: `${size ?? 13}px`, height: `${size ?? 13}px` }" />
</template>

<style scoped>
.spin {
  display: inline-block;
  border: 1.6px solid currentColor;
  border-right-color: transparent;
  border-radius: 50%;
  opacity: 0.85;
  animation: turn 0.7s linear infinite;
}

@keyframes turn {
  to {
    transform: rotate(360deg);
  }
}
</style>
