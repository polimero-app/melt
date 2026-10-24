<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { PhArrowClockwise, PhPlug, PhPower } from '@phosphor-icons/vue'
import type { MessageKey } from '../i18n'
import type { PrinterBadge } from '../monitoring'
import { commandDetail, commandMessage, type Translate } from '../errors'
import { isStale, offConfirmation, snapshotPlug, type PlugObservation, type PlugSummary, type PowerState, type RelayState } from '../plugs'
import Button from './Button.vue'
import StatusBadge from './StatusBadge.vue'
import ConfirmDialog from './ConfirmDialog.vue'

const props = withDefaults(defineProps<{
  plug: PlugSummary
  observation?: PlugObservation
  /** Known activity by lowercase printer name: printing, paused, heating or drying. */
  activity: Record<string, string>
  t: Translate
  locale: string
  showName?: boolean
  /** Reloads the plug list so the Off dialog names current attachments. */
  refresh: () => Promise<PlugSummary[]>
}>(), { showName: true })

// After a confirmed "on" the parent restarts the affected visible camera viewers.
const emit = defineEmits<{ changed: [state: PowerState] }>()

const busy = ref(false)
const confirmOpen = ref(false)
const confirmedPlug = ref(snapshotPlug(props.plug))
const error = ref<string>()

const relayLabel = (relay: RelayState) => props.t(relay === 'on' ? 'plugsView.relayOn' : relay === 'off' ? 'plugsView.relayOff' : 'plugsView.relayUnknown')

// Reuses the printer badge palette: on reads green, off neutral, unknown amber.
const relayBadge: Record<RelayState, PrinterBadge> = { on: 'idle', off: 'unknown', unknown: 'reconnecting' }

const metaLine = computed(() => {
  const observation = props.observation
  if (!observation) return props.t('plugsView.noReading')
  const parts: string[] = []
  if (isStale(observation) && observation.last) {
    parts.push(`${props.t('plugsView.lastKnown', { state: relayLabel(observation.last.state) })} (${props.t('plugsView.stale')})`)
  }
  parts.push(props.t(`plugsView.connection.${observation.connection}` as MessageKey))
  if (observation.last) {
    const date = new Date(observation.last.observedAt)
    // Today's readings show only the time; older ones keep the date.
    const today = date.toDateString() === new Date().toDateString()
    const when = Number.isNaN(date.getTime()) ? observation.last.observedAt : new Intl.DateTimeFormat(props.locale, today ? { timeStyle: 'short' } : { dateStyle: 'medium', timeStyle: 'short' }).format(date)
    parts.push(props.t('plugsView.observed', { time: when }))
  }
  return parts.join(' · ')
})

const attached = computed(() => [...props.plug.printers, ...props.plug.cameras].join(', ') || props.t('plugsView.attachedNone'))

const confirmation = computed(() => {
  const { printers, cameras, active } = offConfirmation(confirmedPlug.value, props.activity)
  const none = props.t('plugControl.none')
  const printerList = printers.map((name) => (active.includes(name) ? `${name} (${props.t(`plugControl.activity.${props.activity[name.toLowerCase()]}` as MessageKey)})` : name))
  return props.t('plugControl.offDescription', {
    printers: printerList.join(', ') || none,
    cameras: cameras.join(', ') || none,
  })
})

async function setPower(state: PowerState, expected = snapshotPlug(props.plug)) {
  if (busy.value) return
  busy.value = true
  error.value = undefined
  try {
    await invoke('plug_set_power', { expected, state })
    // Never optimistic: the displayed relay comes from the device reading.
    await invoke('plug_status', { id: expected.id }).catch(() => undefined)
    if (state === 'on') emit('changed', state)
  } catch (reason) {
    const code = (reason as { code?: string } | null)?.code
    error.value = [commandMessage(reason, props.t), commandDetail(reason)].filter(Boolean).join(' ')
    if (code === 'plugPowerMismatch') await invoke('plug_status', { id: expected.id }).catch(() => undefined)
  } finally {
    busy.value = false
  }
}

async function checkStatus() {
  if (busy.value) return
  busy.value = true
  error.value = undefined
  try {
    await invoke('plug_status', { id: props.plug.id })
  } catch (reason) {
    error.value = [commandMessage(reason, props.t), commandDetail(reason)].filter(Boolean).join(' ')
  } finally {
    busy.value = false
  }
}

async function askOff() {
  if (busy.value) return
  busy.value = true
  error.value = undefined
  try {
    const id = props.plug.id
    const current = (await props.refresh()).find((plug) => plug.id === id)
    if (!current) throw { code: 'plugNotFound' }
    confirmedPlug.value = snapshotPlug(current)
    confirmOpen.value = true
  } catch (reason) {
    error.value = [commandMessage(reason, props.t), commandDetail(reason)].filter(Boolean).join(' ')
  } finally {
    busy.value = false
  }
}

function confirmOff() {
  confirmOpen.value = false
  void setPower('off', confirmedPlug.value)
}
</script>

<template>
  <div class="min-w-0">
    <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
      <div class="min-w-0">
        <div class="flex items-center gap-2">
          <PhPlug v-if="showName" class="size-4 shrink-0 text-gray-500 dark:text-gray-400" aria-hidden="true" />
          <span v-if="showName" class="truncate text-sm font-semibold text-gray-900 dark:text-white">{{ plug.name }}</span>
          <StatusBadge v-if="observation" :status="relayBadge[observation.relay]" :label="relayLabel(observation.relay)" />
          <span v-if="observation && observation.powerWatts !== null" class="text-sm font-semibold tabular-nums text-gray-900 dark:text-white">{{ t('plugsView.power', { watts: observation.powerWatts }) }}</span>
        </div>
        <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ attached }}</p>
        <p class="mt-0.5 text-xs" :class="observation && observation.connection !== 'online' ? 'text-amber-700 dark:text-amber-400' : 'text-gray-500 dark:text-gray-400'">{{ metaLine }}</p>
      </div>
      <div class="flex flex-wrap gap-2">
        <!-- Unknown relay shows both: a guess could hide the action the user needs. -->
        <Button v-if="observation?.relay !== 'on'" variant="secondary" :disabled="busy" :aria-label="t('plugControl.turnOnLabel', { name: plug.name })" @click="setPower('on')"><PhPower class="size-4" aria-hidden="true" /> {{ t('plugControl.on') }}</Button>
        <Button v-if="observation?.relay !== 'off'" variant="danger" :disabled="busy" :aria-label="t('plugControl.turnOffLabel', { name: plug.name })" @click="askOff"><PhPower class="size-4" aria-hidden="true" /> {{ t('plugControl.off') }}</Button>
        <Button variant="secondary" :disabled="busy" :aria-label="t('plugControl.checkStatusLabel', { name: plug.name })" @click="checkStatus"><PhArrowClockwise class="size-4" aria-hidden="true" /> {{ t('plugControl.checkStatus') }}</Button>
      </div>
    </div>
    <p v-if="busy" class="mt-1 text-xs text-gray-500 dark:text-gray-400" role="status">{{ t('plugControl.working') }}</p>
    <p v-if="error" class="mt-1 text-xs text-red-600 dark:text-red-400" role="alert">{{ error }}</p>
    <ConfirmDialog
      :open="confirmOpen"
      :title="t('plugControl.offTitle', { name: confirmedPlug.name })"
      :description="confirmation"
      :confirm-label="t('plugControl.off')"
      :cancel-label="t('common.cancel')"
      confirm-variant="danger"
      @close="confirmOpen = false"
      @confirm="confirmOff"
    />
  </div>
</template>
