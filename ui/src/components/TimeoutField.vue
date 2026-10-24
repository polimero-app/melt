<script setup lang="ts">
import { computed, ref, watchEffect } from 'vue'
import { isValidTimeout } from '../forms'
import type { Translate } from '../errors'

const props = defineProps<{ id: string; t: Translate }>()
const model = defineModel<string>({ required: true })
const input = ref<HTMLInputElement>()
const valid = computed(() => isValidTimeout(model.value))

watchEffect(() => {
  input.value?.setCustomValidity(valid.value ? '' : props.t('addition.timeoutInvalid'))
})
</script>

<template>
  <div>
    <label :for="id" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.timeout') }}</label>
    <input
      :id="id"
      ref="input"
      v-model="model"
      :name="id"
      required
      placeholder="10s"
      autocomplete="off"
      :aria-invalid="!valid"
      :aria-describedby="`${id}-hint${valid ? '' : ` ${id}-error`}`"
      class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10"
    />
    <p :id="`${id}-hint`" class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ t('addition.timeoutHint') }}</p>
    <p v-if="!valid" :id="`${id}-error`" class="mt-1 text-xs text-red-600 dark:text-red-400" role="alert">{{ t('addition.timeoutInvalid') }}</p>
  </div>
</template>
