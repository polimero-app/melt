<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Dialog, DialogPanel, DialogTitle } from "@headlessui/vue";
import { invoke } from "@tauri-apps/api/core";

type AppInfo = {
  version: string;
  modes: [string, string];
};

type Printer = {
  name: string;
  driver: string;
  host: string;
};

type Driver = {
  name: string;
  description: string;
};

const info = ref<AppInfo>();
const printers = ref<Printer[]>([]);
const drivers = ref<Driver[]>([]);
const error = ref<string>();
const aboutOpen = ref(false);
const status = computed(() => (error.value ? "offline" : info.value ? "ready" : "connecting"));

onMounted(async () => {
  try {
    [info.value, printers.value, drivers.value] = await Promise.all([
      invoke<AppInfo>("app_info"),
      invoke<Printer[]>("configured_printers"),
      invoke<Driver[]>("registered_drivers")
    ]);
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
        <p class="lede">The Tauri shell reads your local printer configuration through the shared Rust core.</p>
        <dl>
          <div><dt>Configured</dt><dd>{{ printers.length }} printer{{ printers.length === 1 ? "" : "s" }}</dd></div>
          <div><dt>Core</dt><dd>Shared Rust domain</dd></div>
          <div><dt>GUI</dt><dd>Vue desktop surface</dd></div>
        </dl>
        <div v-if="printers.length" class="printers" aria-label="Configured printers">
          <article v-for="printer in printers" :key="printer.name">
            <p>{{ printer.name }}</p>
            <span>{{ printer.driver }} · {{ printer.host }}</span>
          </article>
        </div>
        <p v-else class="empty">No printers configured yet. Add one with the headless CLI while the setup flow is migrated.</p>
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
          <DialogTitle>Available printer drivers</DialogTitle>
          <p>This desktop process invokes the Rust core directly; it does not run a local server.</p>
          <ul class="drivers">
            <li v-for="driver in drivers" :key="driver.name">
              <strong>{{ driver.name }}</strong>
              <span>{{ driver.description }}</span>
            </li>
          </ul>
          <button type="button" @click="aboutOpen = false">Close</button>
        </DialogPanel>
      </div>
    </Dialog>
  </main>
</template>
