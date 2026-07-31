<script setup lang="ts">
import { Menu, MenuButton, MenuItems, MenuItem } from '@headlessui/vue'
import { computed, type Component } from 'vue'
import { PhDotsThree } from '@phosphor-icons/vue'

export interface ActionMenuItem {
  label: string
  icon: Component
  danger?: boolean
  onSelect: () => void
}

const props = defineProps<{ items: ActionMenuItem[]; label: string }>()

// ponytail: the block separates menu sections with `divide-y` over `py-1` groups;
// destructive actions are the only section split this app needs.
const groups = computed(() =>
  [props.items.filter((item) => !item.danger), props.items.filter((item) => item.danger)].filter(
    (group) => group.length,
  ),
)
</script>

<template>
  <Menu as="div" class="relative inline-block text-left">
    <MenuButton
      class="inline-grid size-8 shrink-0 place-items-center rounded-md text-gray-500 transition hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-white/10 dark:hover:text-white"
      :title="label"
      :aria-label="label"
    >
      <PhDotsThree class="size-5" />
    </MenuButton>
    <transition
      enter-active-class="transition ease-out duration-100"
      enter-from-class="transform opacity-0 scale-95"
      enter-to-class="transform scale-100"
      leave-active-class="transition ease-in duration-75"
      leave-from-class="transform scale-100"
      leave-to-class="transform opacity-0 scale-95"
    >
      <MenuItems
        class="absolute right-0 z-30 mt-2 w-56 origin-top-right divide-y divide-gray-100 rounded-md bg-white shadow-lg outline-1 outline-black/5 dark:divide-white/10 dark:bg-gray-800 dark:shadow-none dark:-outline-offset-1 dark:outline-white/10"
      >
        <div v-for="(group, index) in groups" :key="index" class="py-1">
          <MenuItem v-for="item in group" :key="item.label" v-slot="{ active }">
            <button
              type="button"
              class="flex w-full items-center px-4 py-2 text-left text-sm whitespace-nowrap"
              :class="
                item.danger
                  ? active
                    ? 'bg-red-50 text-red-800 outline-hidden dark:bg-red-500/10 dark:text-red-300'
                    : 'text-red-700 dark:text-red-400'
                  : active
                    ? 'bg-gray-100 text-gray-900 outline-hidden dark:bg-white/5 dark:text-white'
                    : 'text-gray-700 dark:text-gray-300'
              "
              @click="item.onSelect"
            >
              <component
                :is="item.icon"
                class="mr-3 size-5"
                :class="
                  item.danger
                    ? ''
                    : active
                      ? 'text-gray-500 dark:text-white'
                      : 'text-gray-400 dark:text-gray-500'
                "
                aria-hidden="true"
              />
              {{ item.label }}
            </button>
          </MenuItem>
        </div>
      </MenuItems>
    </transition>
  </Menu>
</template>
