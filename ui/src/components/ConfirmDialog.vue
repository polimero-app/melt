<script setup lang="ts">
import { ref, watchEffect } from 'vue'
import { Dialog, DialogDescription, DialogPanel, DialogTitle, TransitionChild, TransitionRoot } from '@headlessui/vue'
import Button from './Button.vue'

const props = withDefaults(defineProps<{
  open: boolean
  title: string
  description?: string
  confirmLabel: string
  cancelLabel: string
  confirmVariant?: 'primary' | 'danger'
  busy?: boolean
  emergency?: boolean
}>(), { confirmVariant: 'primary' })

defineEmits<{ close: []; confirm: [] }>()

const confirmButton = ref<InstanceType<typeof Button>>()
const initialFocus = ref<HTMLElement>()

watchEffect(() => {
  // Keep the focus target stable through the leave transition.
  if (props.open) initialFocus.value = props.emergency ? confirmButton.value?.$el : undefined
})
</script>

<template>
  <TransitionRoot as="template" :show="open">
    <Dialog class="relative z-50" role="alertdialog" :initial-focus="initialFocus" @close="!busy && $emit('close')">
      <TransitionChild as="template" enter="ease-out duration-300" enter-from="opacity-0" enter-to="opacity-100" leave="ease-in duration-200" leave-from="opacity-100" leave-to="opacity-0">
        <div class="fixed inset-0 bg-gray-500/75 transition-opacity dark:bg-gray-950/70" />
      </TransitionChild>

      <div class="fixed inset-0 overflow-y-auto overscroll-contain">
        <div class="flex min-h-full items-center justify-center p-4 pb-[calc(1rem+env(safe-area-inset-bottom))] text-center">
          <TransitionChild
            as="template"
            enter="ease-out duration-300"
            enter-from="opacity-0 translate-y-2 scale-95"
            enter-to="opacity-100 translate-y-0 scale-100"
            leave="ease-in duration-200"
            leave-from="opacity-100 translate-y-0 scale-100"
            leave-to="opacity-0 translate-y-2 scale-95"
          >
            <DialogPanel class="w-full max-w-md transform overflow-hidden rounded-lg bg-white p-6 text-left shadow-xl outline outline-gray-200 transition dark:bg-gray-800 dark:shadow-none dark:outline-white/10">
              <DialogTitle class="text-base font-semibold text-gray-900 dark:text-white">{{ title }}</DialogTitle>
              <DialogDescription v-if="description" class="mt-2 text-sm text-gray-600 dark:text-gray-300">{{ description }}</DialogDescription>
              <div v-if="$slots.default" class="mt-4"><slot /></div>
              <div class="mt-6 flex justify-end gap-2">
                <Button :disabled="busy" @click="$emit('close')">{{ cancelLabel }}</Button>
                <Button ref="confirmButton" :variant="emergency ? 'emergency' : confirmVariant" :disabled="busy" @click="$emit('confirm')">{{ confirmLabel }}</Button>
              </div>
            </DialogPanel>
          </TransitionChild>
        </div>
      </div>
    </Dialog>
  </TransitionRoot>
</template>
