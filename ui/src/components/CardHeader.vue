<script setup lang="ts">
import type { Component } from 'vue'

withDefaults(defineProps<{ title: string; icon?: Component; iconClass?: string }>(), {
  iconClass: 'text-cyan-600 dark:text-cyan-400',
})
</script>

<template>
  <div
    class="flex min-h-12 items-center gap-2.5 border-b border-gray-200 px-4 sm:px-6 dark:border-white/10"
    :class="$slots.default ? 'justify-between' : ''"
  >
    <div class="flex items-center gap-3">
      <component :is="icon" v-if="icon" class="size-4" :class="iconClass" aria-hidden="true" />
      <h3 class="text-sm font-semibold text-gray-900 dark:text-white">{{ title }}</h3>
      <slot name="suffix" />
    </div>
    <div v-if="$slots.default" class="flex items-center gap-1">
      <slot />
    </div>
  </div>
</template>
