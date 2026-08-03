<script setup lang="ts">
type PrinterStatus = 'idle' | 'busy' | 'connecting' | 'synchronizing' | 'reconnecting' | 'offline' | 'unknown'

const props = defineProps<{ status: PrinterStatus; label?: string }>()

const styles: Record<PrinterStatus, string> = {
  idle: 'bg-green-100 text-green-700 dark:bg-green-400/10 dark:text-green-400',
  busy: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-400/10 dark:text-yellow-500',
  connecting: 'bg-gray-100 text-gray-600 dark:bg-gray-400/10 dark:text-gray-400',
  synchronizing: 'bg-cyan-100 text-cyan-800 dark:bg-cyan-400/10 dark:text-cyan-400',
  reconnecting: 'bg-amber-100 text-amber-800 dark:bg-amber-400/10 dark:text-amber-400',
  offline: 'bg-red-100 text-red-700 dark:bg-red-400/10 dark:text-red-400',
  unknown: 'bg-gray-100 text-gray-600 dark:bg-gray-400/10 dark:text-gray-400',
}

const dotStyles: Record<PrinterStatus, string> = {
  idle: 'fill-green-500 dark:fill-green-400',
  busy: 'fill-yellow-500 dark:fill-yellow-400',
  connecting: 'fill-gray-400 dark:fill-gray-500',
  synchronizing: 'fill-cyan-500 dark:fill-cyan-400',
  reconnecting: 'fill-amber-500 dark:fill-amber-400',
  offline: 'fill-red-500 dark:fill-red-400',
  unknown: 'fill-gray-400 dark:fill-gray-500',
}

const labels: Record<PrinterStatus, string> = {
  idle: 'Online · idle',
  busy: 'Online · busy',
  connecting: 'Connecting',
  synchronizing: 'Synchronizing',
  reconnecting: 'Reconnecting',
  offline: 'Offline',
  unknown: 'Unknown',
}
</script>

<template>
  <span
    class="inline-flex items-center gap-x-1.5 rounded-md px-2 py-1 text-xs font-medium"
    :class="styles[props.status]"
  >
    <svg class="size-1.5" :class="dotStyles[props.status]" viewBox="0 0 6 6" aria-hidden="true">
      <circle cx="3" cy="3" r="3" />
    </svg>
    {{ props.label ?? labels[props.status] }}
  </span>
</template>
