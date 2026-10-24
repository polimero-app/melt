<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{ text: string }>()
// Keeps the tail (extension and distinguishing suffix) visible when the name is clipped.
const tailLength = 12
const parts = computed(() => {
  const chars = Array.from(props.text)
  return [chars.slice(0, -tailLength).join(''), chars.slice(-tailLength).join('')]
})
</script>

<template>
  <span class="flex min-w-0"><span class="truncate">{{ parts[0] }}</span><span class="shrink-0 whitespace-pre">{{ parts[1] }}</span></span>
</template>
