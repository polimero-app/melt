<script setup lang="ts">
withDefaults(defineProps<{ variant?: 'primary' | 'secondary' | 'danger'; type?: 'button' | 'submit' }>(), {
  variant: 'secondary',
  type: 'button',
})

const variantClasses: Record<string, string> = {
  primary:
    'bg-cyan-600 text-white shadow-xs hover:bg-cyan-500 dark:bg-cyan-500 dark:shadow-none dark:hover:bg-cyan-400',
  secondary:
    'bg-white text-gray-900 shadow-xs inset-ring inset-ring-gray-300 hover:bg-gray-50 dark:bg-white/10 dark:text-white dark:shadow-none dark:inset-ring-white/5 dark:hover:bg-white/20',
  danger:
    'bg-red-50 text-red-700 shadow-xs inset-ring inset-ring-red-600/20 hover:bg-red-100 dark:bg-red-500/10 dark:text-red-400 dark:shadow-none dark:inset-ring-red-500/20 dark:hover:bg-red-500/20',
}
</script>

<template>
  <button
    :type="type"
    class="inline-flex items-center justify-center gap-x-1.5 rounded-md px-2.5 py-1.5 text-sm font-semibold transition disabled:cursor-not-allowed disabled:opacity-35"
    :class="variantClasses[variant]"
  >
    <slot />
  </button>
</template>
