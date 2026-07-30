<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Dialog, DialogPanel, DialogTitle } from "@headlessui/vue";
import { invoke } from "@tauri-apps/api/core";
import { preferredLocale, translate, type Locale } from "./i18n";

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

type NewPrinter = Printer & {
  serial: string;
  timeout: string;
  insecure: boolean;
};

const info = ref<AppInfo>();
const printers = ref<Printer[]>([]);
const drivers = ref<Driver[]>([]);
const loading = ref(true);
const loadError = ref<string>();
const profileError = ref<string>();
const aboutOpen = ref(false);
const additionOpen = ref(false);
const adding = ref(false);
const additionError = ref<string>();
const removalOpen = ref(false);
const selectedPrinter = ref<Printer>();
const removing = ref(false);
const removalError = ref<string>();
const locale = ref<Locale>(preferredLocale());
const status = computed(() => (loadError.value ? "offline" : info.value ? "ready" : "connecting"));
const statusKeys = {
  connecting: "status.connecting",
  ready: "status.ready",
  offline: "status.offline"
} as const;
const statusLabel = computed(() => t(statusKeys[status.value]));
const printerCount = computed(() => printers.value.length);
const draft = ref({
  name: "",
  driver: "",
  host: "",
  serial: "",
  timeout: "10s",
  insecure: false,
  accessCode: ""
});

function t(key: Parameters<typeof translate>[1], values?: Record<string, string | number>) {
  return translate(locale.value, key, values);
}

function message(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}

async function load() {
  loading.value = true;
  loadError.value = undefined;
  profileError.value = undefined;

  const [appInfo, profiles, availableDrivers] = await Promise.allSettled([
    invoke<AppInfo>("app_info"),
    invoke<Printer[]>("configured_printers"),
    invoke<Driver[]>("registered_drivers")
  ]);

  if (appInfo.status === "fulfilled") info.value = appInfo.value;
  else loadError.value = message(appInfo.reason);

  if (profiles.status === "fulfilled") printers.value = profiles.value;
  else profileError.value = message(profiles.reason);

  if (availableDrivers.status === "fulfilled") drivers.value = availableDrivers.value;
  else loadError.value ??= message(availableDrivers.reason);

  loading.value = false;
}

onMounted(load);

function openRemoval(printer: Printer) {
  selectedPrinter.value = printer;
  removalError.value = undefined;
  removalOpen.value = true;
}

function closeRemoval() {
  if (!removing.value) removalOpen.value = false;
}

function openAddition() {
  draft.value = {
    name: "",
    driver: drivers.value.find((driver) => driver.name === "moonraker")?.name ?? drivers.value[0]?.name ?? "",
    host: "",
    serial: "",
    timeout: "10s",
    insecure: false,
    accessCode: ""
  };
  additionError.value = undefined;
  additionOpen.value = true;
}

function closeAddition() {
  if (!adding.value) additionOpen.value = false;
}

async function addPrinter() {
  if (adding.value) return;
  adding.value = true;
  additionError.value = undefined;

  try {
    const profile = await invoke<NewPrinter>("create_configured_printer", { request: draft.value });
    printers.value.push({ name: profile.name, driver: profile.driver, host: profile.host });
    additionOpen.value = false;
    draft.value.accessCode = "";
  } catch (reason) {
    additionError.value = message(reason);
  } finally {
    adding.value = false;
  }
}

async function removePrinter() {
  if (!selectedPrinter.value || removing.value) return;
  removing.value = true;
  removalError.value = undefined;

  try {
    await invoke("remove_configured_printer", { name: selectedPrinter.value.name });
    printers.value = printers.value.filter((printer) => printer.name !== selectedPrinter.value?.name);
    removalOpen.value = false;
  } catch (reason) {
    removalError.value = message(reason);
  } finally {
    removing.value = false;
  }
}
</script>

<template>
  <main class="shell">
    <header class="masthead">
      <div>
        <p class="eyebrow">{{ t("app.eyebrow") }}</p>
        <h1>{{ t("app.title") }}</h1>
      </div>
      <div class="masthead-actions">
        <label class="locale">
          <span class="sr-only">{{ t("app.locale") }}</span>
          <select v-model="locale">
            <option value="en">{{ t("app.english") }}</option>
            <option value="pt-BR">{{ t("app.portuguese") }}</option>
          </select>
        </label>
        <button class="status" type="button" :aria-label="t('app.status', { status: statusLabel })" @click="aboutOpen = true">
        <span :class="['signal', status]" />
        {{ statusLabel }}
        </button>
      </div>
    </header>

    <section class="workspace" aria-labelledby="migration-title">
      <div class="rail" aria-hidden="true">
        <span>01</span><i /><span>02</span><i /><span>03</span>
      </div>
      <div class="panel">
        <p class="eyebrow">{{ t("workspace.eyebrow") }}</p>
        <h2 id="migration-title">{{ t("workspace.title") }}</h2>
        <p class="lede">{{ t("workspace.description") }}</p>
        <dl>
          <div><dt>{{ t("workspace.configured") }}</dt><dd>{{ printerCount }} {{ t(printerCount === 1 ? "workspace.printer" : "workspace.printers") }}</dd></div>
          <div><dt>{{ t("workspace.core") }}</dt><dd>{{ t("workspace.coreValue") }}</dd></div>
          <div><dt>{{ t("workspace.gui") }}</dt><dd>{{ t("workspace.guiValue") }}</dd></div>
        </dl>
        <div v-if="loading" class="loading" role="status" aria-live="polite">
          {{ t("profiles.loading") }}
        </div>
        <div v-else-if="profileError" class="notice error-notice" role="alert">
          <strong>{{ t("profiles.error") }}</strong>
          <span>{{ profileError }}</span>
          <button type="button" @click="load">{{ t("common.retry") }}</button>
        </div>
        <div v-else-if="printers.length" class="printers" :aria-label="t('profiles.label')" :aria-busy="loading">
          <article v-for="printer in printers" :key="printer.name">
            <p>{{ printer.name }}</p>
            <span>{{ printer.driver }} · {{ printer.host }}</span>
            <button type="button" @click="openRemoval(printer)">{{ t("profiles.remove") }}</button>
          </article>
        </div>
        <p v-else class="empty">{{ t("profiles.empty") }}</p>
        <button class="add-printer" type="button" :disabled="!drivers.length" @click="openAddition">{{ t("profiles.add") }}</button>
      </div>
    </section>

    <footer>
      <span>{{ info ? `v${info.version}` : t("footer.starting") }}</span>
      <span v-if="loadError" class="error" role="alert">{{ loadError }}</span>
      <span v-else>{{ t("footer.private") }}</span>
    </footer>

    <Dialog :open="aboutOpen" @close="aboutOpen = false" class="dialog">
      <div class="backdrop" aria-hidden="true" />
      <div class="dialog-frame">
        <DialogPanel class="dialog-panel">
          <DialogTitle>{{ t("drivers.title") }}</DialogTitle>
          <p>{{ t("drivers.description") }}</p>
          <ul class="drivers">
            <li v-for="driver in drivers" :key="driver.name">
              <strong>{{ driver.name }}</strong>
              <span>{{ driver.description }}</span>
            </li>
          </ul>
          <p v-if="!drivers.length" class="empty">{{ t("drivers.empty") }}</p>
          <button type="button" @click="aboutOpen = false">{{ t("common.close") }}</button>
        </DialogPanel>
      </div>
    </Dialog>

    <Dialog :open="additionOpen" @close="closeAddition" class="dialog">
      <div class="backdrop" aria-hidden="true" />
      <div class="dialog-frame">
        <DialogPanel class="dialog-panel">
          <DialogTitle>{{ t("addition.title") }}</DialogTitle>
          <p>{{ t("addition.description") }}</p>
          <form class="profile-form" @submit.prevent="addPrinter">
            <label>{{ t("addition.name") }}<input v-model="draft.name" required maxlength="64" autocomplete="off" /></label>
            <label>{{ t("addition.driver") }}<select v-model="draft.driver" required><option v-for="driver in drivers" :key="driver.name" :value="driver.name">{{ driver.name }}</option></select></label>
            <label>{{ t("addition.host") }}<input v-model="draft.host" required autocomplete="off" /></label>
            <label>{{ t("addition.serial") }}<input v-model="draft.serial" autocomplete="off" /></label>
            <label>{{ t("addition.timeout") }}<input v-model="draft.timeout" required /></label>
            <label>{{ t("addition.accessCode") }}<input v-model="draft.accessCode" type="password" autocomplete="new-password" /></label>
            <label class="checkbox"><input v-model="draft.insecure" type="checkbox" />{{ t("addition.insecure") }}</label>
            <p v-if="additionError" class="dialog-error" role="alert"><strong>{{ t("addition.error") }}</strong> {{ additionError }}</p>
            <div class="actions">
              <button type="button" :disabled="adding" @click="closeAddition">{{ t("common.cancel") }}</button>
              <button type="submit" :disabled="adding">{{ adding ? t("common.adding") : t("addition.confirm") }}</button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>

    <Dialog :open="removalOpen" @close="closeRemoval" class="dialog">
      <div class="backdrop" aria-hidden="true" />
      <div class="dialog-frame">
        <DialogPanel class="dialog-panel">
          <DialogTitle>{{ t("removal.title", { name: selectedPrinter?.name ?? "" }) }}</DialogTitle>
          <p>{{ t("removal.description") }}</p>
          <p v-if="removalError" class="dialog-error" role="alert">
            <strong>{{ t("removal.error") }}</strong> {{ removalError }}
          </p>
          <div class="actions">
            <button type="button" :disabled="removing" @click="closeRemoval">{{ t("common.cancel") }}</button>
            <button class="danger" type="button" :disabled="removing" @click="removePrinter">
              {{ removing ? t("common.removing") : t("removal.confirm") }}
            </button>
          </div>
        </DialogPanel>
      </div>
    </Dialog>
  </main>
</template>
