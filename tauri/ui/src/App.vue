<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Dialog, DialogPanel, DialogTitle } from "@headlessui/vue";
import { invoke } from "@tauri-apps/api/core";

type AppInfo = {
  version: string;
  modes: [string, string];
};

const info = ref<AppInfo>();
const error = ref<string>();
const aboutOpen = ref(false);
const status = computed(() => (error.value ? "offline" : info.value ? "ready" : "connecting"));

onMounted(async () => {
  try {
    info.value = await invoke<AppInfo>("app_info");
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason);
  }
});
</script>

<template>
  <main class="shell">
    <header class="masthead">
      <div>
        <p class="eyebrow">LOCAL-FIRST PRINT CONTROL</p>
        <h1>POLIMERO</h1>
      </div>
      <button class="status" type="button" @click="aboutOpen = true">
        <span :class="['signal', status]" />
        {{ status }}
      </button>
    </header>

    <section class="workspace" aria-labelledby="migration-title">
      <div class="rail" aria-hidden="true">
        <span>01</span><i /><span>02</span><i /><span>03</span>
      </div>
      <div class="panel">
        <p class="eyebrow">RUST + TAURI V2</p>
        <h2 id="migration-title">Desktop control is coming online.</h2>
        <p class="lede">
          The Tauri shell is connected to the shared Rust core. Printer controls
          arrive only after the existing CLI contract is reproduced.
        </p>
        <dl>
          <div><dt>Core</dt><dd>Shared Rust domain</dd></div>
          <div><dt>CLI</dt><dd>Contract migration</dd></div>
          <div><dt>GUI</dt><dd>Vue desktop surface</dd></div>
        </dl>
      </div>
    </section>

    <footer>
      <span>{{ info ? `v${info.version}` : "starting core…" }}</span>
      <span v-if="error" class="error">{{ error }}</span>
      <span v-else>no cloud · no telemetry</span>
    </footer>

    <Dialog :open="aboutOpen" @close="aboutOpen = false" class="dialog">
      <div class="backdrop" aria-hidden="true" />
      <div class="dialog-frame">
        <DialogPanel class="dialog-panel">
          <DialogTitle>Polimero migration shell</DialogTitle>
          <p>This desktop process invokes the Rust core directly; it does not run a local server.</p>
          <button type="button" @click="aboutOpen = false">Close</button>
        </DialogPanel>
      </div>
    </Dialog>
  </main>
</template>

