<script setup lang="ts">
import { badgeFillClasses, badgeSurfaceClasses } from '../presentation'
import type { PrinterBadge } from '../monitoring'

const props = defineProps<{ status: PrinterBadge; label: string }>()
</script>

<template>
  <span
    class="inline-flex items-center gap-x-1.5 rounded-md px-2 py-1 text-xs font-medium"
    :class="badgeSurfaceClasses[props.status]"
  >
    <svg class="size-1.5" :class="badgeFillClasses[props.status]" viewBox="0 0 6 6" aria-hidden="true">
      <circle cx="3" cy="3" r="3" />
    </svg>
    {{ props.label }}
  </span>
</template>
