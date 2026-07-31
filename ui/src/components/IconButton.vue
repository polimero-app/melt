<script setup lang="ts">
withDefaults(defineProps<{ variant?: 'ghost' | 'outline'; type?: 'button' | 'submit' }>(), {
  variant: 'ghost',
  type: 'button',
})

const variantClasses: Record<string, string> = {
  ghost:
    'size-8 rounded-md text-gray-500 hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-white/10 dark:hover:text-white',
  outline:
    'min-h-7 min-w-7 rounded-md border border-gray-300 px-1 text-xs font-semibold text-gray-500 hover:border-cyan-500 hover:bg-cyan-50 hover:text-cyan-700 dark:border-white/15 dark:text-gray-400 dark:hover:border-cyan-400/35 dark:hover:bg-cyan-400/10 dark:hover:text-cyan-200',
}
</script>

<template>
  <button
    :type="type"
    class="inline-grid shrink-0 place-items-center transition disabled:cursor-not-allowed disabled:opacity-35"
    :class="variantClasses[variant]"
  >
    <slot />
  </button>
</template>
