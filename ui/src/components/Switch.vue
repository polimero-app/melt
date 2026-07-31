<script setup lang="ts">
defineProps<{ label: string }>()
const model = defineModel<boolean>({ required: true })
</script>

<template>
  <div
    class="group relative inline-flex h-5 w-10 shrink-0 items-center justify-center rounded-full outline-offset-2 outline-cyan-600 has-focus-visible:outline-2 dark:outline-cyan-500"
  >
    <span
      class="absolute mx-auto h-4 w-9 rounded-full bg-gray-200 inset-ring inset-ring-gray-900/5 transition-colors duration-200 ease-in-out group-has-checked:bg-cyan-600 dark:bg-gray-800/50 dark:inset-ring-white/10 dark:group-has-checked:bg-cyan-500"
    ></span>
    <span
      class="absolute left-0 size-5 rounded-full border border-gray-300 bg-white shadow-xs transition-transform duration-200 ease-in-out group-has-checked:translate-x-5 dark:border-transparent dark:shadow-none"
    ></span>
    <input
      v-model="model"
      type="checkbox"
      class="absolute inset-0 size-full appearance-none focus:outline-hidden"
      :aria-label="label"
    />
  </div>
</template>
