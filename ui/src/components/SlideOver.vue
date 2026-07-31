<script setup lang="ts">
import { Dialog, DialogPanel, DialogTitle, TransitionChild, TransitionRoot } from '@headlessui/vue'
import { PhX } from '@phosphor-icons/vue'

withDefaults(defineProps<{ open: boolean; title: string; description?: string; closeLabel?: string }>(), {
  closeLabel: 'Close panel',
})
defineEmits<{ close: [] }>()
</script>

<template>
  <TransitionRoot as="template" :show="open">
    <Dialog class="relative z-40" @close="$emit('close')">
      <TransitionChild as="template" enter="ease-in-out duration-500" enter-from="opacity-0" enter-to="opacity-100" leave="ease-in-out duration-500" leave-from="opacity-100" leave-to="opacity-0">
        <div class="fixed inset-0 bg-gray-500/75 transition-opacity dark:bg-gray-900/50" />
      </TransitionChild>
      <div class="fixed inset-0 overflow-hidden">
        <div class="absolute inset-0 overflow-hidden">
          <div class="pointer-events-none fixed inset-y-0 right-0 flex max-w-full pl-10 sm:pl-16">
            <TransitionChild
              as="template"
              enter="transform transition ease-in-out duration-500 sm:duration-700"
              enter-from="translate-x-full"
              enter-to="translate-x-0"
              leave="transform transition ease-in-out duration-500 sm:duration-700"
              leave-from="translate-x-0"
              leave-to="translate-x-full"
            >
              <DialogPanel class="pointer-events-auto w-screen max-w-md">
                <div class="relative flex h-full flex-col overflow-y-auto bg-white py-6 shadow-xl dark:bg-gray-800 dark:after:absolute dark:after:inset-y-0 dark:after:left-0 dark:after:w-px dark:after:bg-white/10">
                  <div class="px-4 sm:px-6">
                    <div class="flex items-start justify-between">
                      <DialogTitle class="text-base font-semibold text-gray-900 dark:text-white">{{ title }}</DialogTitle>
                      <div class="ml-3 flex h-7 items-center">
                        <button
                          type="button"
                          class="relative rounded-md text-gray-400 hover:text-gray-500 dark:hover:text-white"
                          @click="$emit('close')"
                        >
                          <span class="absolute -inset-2.5"></span>
                          <span class="sr-only">{{ closeLabel }}</span>
                          <PhX class="size-6" aria-hidden="true" />
                        </button>
                      </div>
                    </div>
                    <p v-if="description" class="mt-1 text-sm text-gray-500 dark:text-gray-400">{{ description }}</p>
                  </div>
                  <div class="relative mt-6 flex-1 px-4 sm:px-6">
                    <slot />
                  </div>
                  <div v-if="$slots.footer" class="mt-6 flex justify-end gap-2 border-t border-gray-200 px-4 pt-4 sm:px-6 dark:border-white/10">
                    <slot name="footer" />
                  </div>
                </div>
              </DialogPanel>
            </TransitionChild>
          </div>
        </div>
      </div>
    </Dialog>
  </TransitionRoot>
</template>
