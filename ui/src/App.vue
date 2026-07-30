<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
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

type Capabilities = {
  status: boolean;
  cameraStream: boolean;
  cameraSnapshot: boolean;
  fileList: boolean;
  fileDownload: boolean;
  fileUpload: boolean;
  jobStart: boolean;
  jobPause: boolean;
  jobResume: boolean;
  jobCancel: boolean;
  emergencyStop: boolean;
  temperatureWrite: boolean;
  motionControl: boolean;
  fanControl: boolean;
};

type Temperature = {
  currentCelsius: number;
  targetCelsius?: number;
};

type PrinterStatus = {
  state: "idle" | "printing" | "paused" | "error" | "unknown";
  temperatures?: { nozzle?: Temperature; bed?: Temperature };
  job?: { name: string };
  progress?: { percent: number; currentLayer?: number; totalLayers?: number };
  errors: { code: string; message: string }[];
  warnings: { code: string; message: string }[];
  fans: Record<string, number>;
};

type MonitorEntry = {
  name: string;
  driver: string;
  status?: PrinterStatus;
  error?: string;
};

type FileEntry = {
  name: string;
  devicePath: string;
  type: "file" | "directory";
  sizeBytes?: number;
  modifiedAt?: string;
};

type FileList = {
  entries: FileEntry[];
};

type Diagnostics = {
  version: string;
  platform: string;
  configuredProfiles: number;
  drivers: Record<string, number>;
  identifiersRedacted: boolean;
  monitorWorkers: number;
  monitorIntervalSeconds: number;
  protocolTracesIncluded: boolean;
};

type CameraSnapshot = {
  dataUrl: string;
};

type CameraStream = {
  url: string;
};

const info = ref<AppInfo>();
const printers = ref<Printer[]>([]);
const drivers = ref<Driver[]>([]);
const monitoring = ref<MonitorEntry[]>([]);
const capabilities = ref<Capabilities>();
const files = ref<FileEntry[]>([]);
const loading = ref(true);
const refreshing = ref(false);
const filesLoading = ref(false);
const loadError = ref<string>();
const profileError = ref<string>();
const workspaceError = ref<string>();
const aboutOpen = ref(false);
const additionOpen = ref(false);
const adding = ref(false);
const additionError = ref<string>();
const removalOpen = ref(false);
const selectedPrinter = ref<Printer>();
const removing = ref(false);
const removalError = ref<string>();
const actionOpen = ref(false);
const pendingAction = ref<"start" | "pause" | "resume" | "cancel" | "fan" | "home">();
const pendingDevicePath = ref<string>();
const pendingFanSpeed = ref<number>();
const actionSending = ref(false);
const actionError = ref<string>();
const diagnosticsOpen = ref(false);
const diagnostics = ref<Diagnostics>();
const diagnosticsError = ref<string>();
const cameraUrl = ref<string>();
const cameraLoading = ref(false);
const cameraError = ref<string>();
const locale = ref<Locale>(preferredLocale());
let monitorTimer: number | undefined;

const status = computed(() => (loadError.value ? "offline" : info.value ? "ready" : "connecting"));
const statusKeys = {
  connecting: "status.connecting",
  ready: "status.ready",
  offline: "status.offline"
} as const;
const statusLabel = computed(() => t(statusKeys[status.value]));
const printerCount = computed(() => printers.value.length);
const selectedMonitor = computed(() => monitoring.value.find((entry) => entry.name === selectedPrinter.value?.name));
const selectedStatus = computed(() => selectedMonitor.value?.status);
const selectedError = computed(() => selectedMonitor.value?.error ?? workspaceError.value);
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

  if (profiles.status === "fulfilled") {
    printers.value = profiles.value;
    const retained = profiles.value.find((printer) => printer.name === selectedPrinter.value?.name);
    const printer = retained ?? profiles.value[0];
    if (printer) await selectPrinter(printer, false);
    else {
      selectedPrinter.value = undefined;
      capabilities.value = undefined;
      monitoring.value = [];
      files.value = [];
      cameraUrl.value = undefined;
      cameraError.value = undefined;
    }
  } else profileError.value = message(profiles.reason);

  if (availableDrivers.status === "fulfilled") drivers.value = availableDrivers.value;
  else loadError.value ??= message(availableDrivers.reason);

  loading.value = false;
}

async function refreshMonitoring() {
  if (refreshing.value || !printers.value.length) return;
  refreshing.value = true;
  workspaceError.value = undefined;
  try {
    monitoring.value = await invoke<MonitorEntry[]>("monitored_printers");
  } catch (reason) {
    workspaceError.value = message(reason);
  } finally {
    refreshing.value = false;
  }
}

async function selectPrinter(printer: Printer, refresh = true) {
  selectedPrinter.value = printer;
  capabilities.value = undefined;
  files.value = [];
  cameraUrl.value = undefined;
  cameraError.value = undefined;
  workspaceError.value = undefined;
  try {
    const result = await invoke<{ capabilities: Capabilities }>("printer_capabilities", { name: printer.name });
    capabilities.value = result.capabilities;
  } catch (reason) {
    workspaceError.value = message(reason);
  }
  if (refresh) await refreshMonitoring();
}

async function loadFiles() {
  if (!selectedPrinter.value || filesLoading.value) return;
  filesLoading.value = true;
  workspaceError.value = undefined;
  try {
    files.value = (await invoke<FileList>("printer_files", { name: selectedPrinter.value.name })).entries;
  } catch (reason) {
    workspaceError.value = message(reason);
  } finally {
    filesLoading.value = false;
  }
}

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
    const printer = { name: profile.name, driver: profile.driver, host: profile.host };
    printers.value.push(printer);
    additionOpen.value = false;
    draft.value.accessCode = "";
    await selectPrinter(printer);
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
  const removed = selectedPrinter.value.name;

  try {
    await invoke("remove_configured_printer", { name: removed });
    printers.value = printers.value.filter((printer) => printer.name !== removed);
    monitoring.value = monitoring.value.filter((printer) => printer.name !== removed);
    removalOpen.value = false;
    const next = printers.value[0];
    if (next) await selectPrinter(next, false);
    else {
      selectedPrinter.value = undefined;
      capabilities.value = undefined;
      files.value = [];
      cameraUrl.value = undefined;
      cameraError.value = undefined;
    }
  } catch (reason) {
    removalError.value = message(reason);
  } finally {
    removing.value = false;
  }
}

function openAction(
  action: "start" | "pause" | "resume" | "cancel" | "fan" | "home",
  devicePath?: string
) {
  actionError.value = undefined;
  pendingAction.value = action;
  pendingDevicePath.value = devicePath;
  actionOpen.value = true;
}

function queueFan(event: Event) {
  const speed = Number((event.target as HTMLInputElement).value);
  if (Number.isInteger(speed)) {
    pendingFanSpeed.value = speed;
    openAction("fan");
  }
}

async function adjustTemperature(kind: "nozzleCelsius" | "bedCelsius", delta: number) {
  if (!selectedPrinter.value || !selectedStatus.value) return;
  const temperature = kind === "nozzleCelsius"
    ? selectedStatus.value.temperatures?.nozzle
    : selectedStatus.value.temperatures?.bed;
  if (!temperature) return;
  const maximum = kind === "nozzleCelsius" ? 300 : 120;
  const base = temperature.targetCelsius ?? temperature.currentCelsius;
  const next = Math.max(0, Math.min(maximum, Math.round((base + delta) / 5) * 5));
  workspaceError.value = undefined;
  try {
    await invoke("printer_temperature_set", {
      request: { name: selectedPrinter.value.name, [kind]: next }
    });
    await refreshMonitoring();
  } catch (reason) {
    workspaceError.value = message(reason);
  }
}

async function sendAction() {
  if (!selectedPrinter.value || !pendingAction.value || actionSending.value) return;
  actionSending.value = true;
  actionError.value = undefined;
  try {
    if (pendingAction.value === "fan") {
      await invoke("printer_fan_set", {
        request: { name: selectedPrinter.value.name, fan: "partCooling", speedPercent: pendingFanSpeed.value ?? 0, confirmed: true }
      });
    } else if (pendingAction.value === "home") {
      await invoke("printer_motion_home", {
        request: { name: selectedPrinter.value.name, confirmed: true }
      });
    } else {
      await invoke("printer_job_action", {
        request: {
          name: selectedPrinter.value.name,
          action: pendingAction.value,
          devicePath: pendingDevicePath.value,
          confirmed: true
        }
      });
    }
    actionOpen.value = false;
    await refreshMonitoring();
  } catch (reason) {
    actionError.value = message(reason);
  } finally {
    actionSending.value = false;
  }
}

async function emergencyStop() {
  if (!selectedPrinter.value) return;
  workspaceError.value = undefined;
  try {
    await invoke("printer_emergency_stop", { name: selectedPrinter.value.name });
    await refreshMonitoring();
  } catch (reason) {
    workspaceError.value = message(reason);
  }
}

async function loadCamera() {
  if (!selectedPrinter.value || cameraLoading.value) return;
  cameraLoading.value = true;
  cameraError.value = undefined;
  try {
    if (capabilities.value?.cameraStream) {
      cameraUrl.value = (await invoke<CameraStream>("printer_camera_stream", { name: selectedPrinter.value.name })).url;
    } else if (capabilities.value?.cameraSnapshot) {
      cameraUrl.value = (await invoke<CameraSnapshot>("printer_camera_snapshot", { name: selectedPrinter.value.name })).dataUrl;
    }
  } catch (reason) {
    cameraUrl.value = undefined;
    cameraError.value = message(reason);
  } finally {
    cameraLoading.value = false;
  }
}

async function captureSnapshot() {
  if (!selectedPrinter.value || cameraLoading.value || !capabilities.value?.cameraSnapshot) return;
  cameraLoading.value = true;
  cameraError.value = undefined;
  try {
    cameraUrl.value = (await invoke<CameraSnapshot>("printer_camera_snapshot", { name: selectedPrinter.value.name })).dataUrl;
  } catch (reason) {
    cameraError.value = message(reason);
  } finally {
    cameraLoading.value = false;
  }
}

async function openDiagnostics() {
  diagnosticsOpen.value = true;
  diagnostics.value = undefined;
  diagnosticsError.value = undefined;
  try {
    diagnostics.value = await invoke<Diagnostics>("diagnostics_report");
  } catch (reason) {
    diagnosticsError.value = message(reason);
  }
}

function formatTemperature(temperature?: Temperature) {
  if (!temperature) return "—";
  return `${temperature.currentCelsius.toFixed(1)}°${temperature.targetCelsius === undefined ? "" : ` / ${temperature.targetCelsius.toFixed(1)}°`}`;
}

onMounted(() => {
  void load();
  monitorTimer = window.setInterval(() => void refreshMonitoring(), 5000);
});

onUnmounted(() => {
  if (monitorTimer !== undefined) window.clearInterval(monitorTimer);
});
</script>

<template>
  <main class="shell">
    <header class="masthead">
      <div>
        <p class="eyebrow">{{ t("app.eyebrow") }}</p>
        <h1>{{ t("app.title") }}</h1>
      </div>
      <div class="masthead-actions">
        <button class="diagnostics-button" type="button" @click="openDiagnostics">{{ t("diagnostics.open") }}</button>
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

    <section class="workspace" aria-labelledby="workspace-title">
      <div class="rail" aria-hidden="true">
        <span>01</span><i /><span>02</span><i /><span>03</span>
      </div>
      <div class="panel">
        <p class="eyebrow">{{ selectedPrinter ? t("workspace.monitoring") : t("workspace.eyebrow") }}</p>
        <h2 id="workspace-title">{{ selectedPrinter ? selectedPrinter.name : t("dashboard.emptyTitle") }}</h2>
        <p class="lede">{{ selectedPrinter ? `${selectedPrinter.driver} · ${selectedPrinter.host}` : t("dashboard.emptyDescription") }}</p>

        <dl>
          <div><dt>{{ t("workspace.configured") }}</dt><dd>{{ printerCount }} {{ t(printerCount === 1 ? "workspace.printer" : "workspace.printers") }}</dd></div>
          <div><dt>{{ t("workspace.core") }}</dt><dd>{{ t("workspace.coreValue") }}</dd></div>
          <div><dt>{{ t("workspace.gui") }}</dt><dd>{{ t("workspace.guiValue") }}</dd></div>
        </dl>

        <div v-if="loading" class="loading" role="status" aria-live="polite">{{ t("profiles.loading") }}</div>
        <div v-else-if="profileError" class="notice error-notice" role="alert">
          <strong>{{ t("profiles.error") }}</strong>
          <span>{{ profileError }}</span>
          <button type="button" @click="load">{{ t("common.retry") }}</button>
        </div>
        <div v-else-if="printers.length" class="printers" :aria-label="t('profiles.label')">
          <article v-for="printer in printers" :key="printer.name" :class="{ selected: selectedPrinter?.name === printer.name }">
            <div>
              <p>{{ printer.name }}</p>
              <span>{{ printer.driver }} · {{ printer.host }}</span>
            </div>
            <button class="select-profile" type="button" @click="selectPrinter(printer)">{{ t("profiles.select") }}</button>
            <button type="button" @click="openRemoval(printer)">{{ t("profiles.remove") }}</button>
          </article>
        </div>
        <p v-else class="empty">{{ t("profiles.empty") }}</p>
        <button class="add-printer" type="button" :disabled="!drivers.length" @click="openAddition">{{ t("profiles.add") }}</button>

        <section v-if="selectedPrinter" class="dashboard" :aria-label="t('dashboard.title')">
          <header class="dashboard-header">
            <div>
              <p class="eyebrow">{{ t("dashboard.title") }}</p>
              <span class="monitor-note">{{ t("dashboard.monitoring") }}</span>
            </div>
            <button class="refresh-button" type="button" :disabled="refreshing" @click="refreshMonitoring">
              {{ refreshing ? t("dashboard.refreshing") : t("dashboard.refresh") }}
            </button>
          </header>

          <p v-if="selectedError" class="notice dashboard-notice" role="alert">
            <strong>{{ t("dashboard.monitoringError") }}</strong> {{ selectedError }}
          </p>

          <div v-else-if="selectedStatus" class="telemetry-grid">
            <article class="telemetry-state">
              <span>{{ t("dashboard.state") }}</span>
              <strong>{{ selectedStatus.state }}</strong>
            </article>
            <article>
              <span>{{ t("dashboard.job") }}</span>
              <strong>{{ selectedStatus.job?.name ?? t("dashboard.noJob") }}</strong>
            </article>
            <article>
              <span>{{ t("dashboard.progress") }}</span>
              <strong>{{ selectedStatus.progress ? `${selectedStatus.progress.percent}%` : "—" }}</strong>
            </article>
            <article>
              <span>{{ t("dashboard.temperature") }}</span>
              <strong>{{ formatTemperature(selectedStatus.temperatures?.nozzle) }} / {{ formatTemperature(selectedStatus.temperatures?.bed) }}</strong>
            </article>
          </div>

          <div class="capability-grid">
            <article v-if="capabilities?.cameraStream || capabilities?.cameraSnapshot" class="capability-card camera-card">
              <p class="eyebrow">CAMERA</p>
              <img v-if="cameraUrl" :src="cameraUrl" :alt="t('dashboard.cameraPreview')" />
              <span v-else>{{ t("dashboard.cameraPreview") }}</span>
              <span v-if="cameraError" class="error">{{ cameraError }}</span>
              <div class="control-actions">
                <button type="button" :disabled="cameraLoading" @click="loadCamera">{{ cameraLoading ? t("common.loading") : t("dashboard.cameraStream") }}</button>
                <button v-if="capabilities.cameraSnapshot" type="button" :disabled="cameraLoading" @click="captureSnapshot">{{ t("dashboard.cameraSnapshot") }}</button>
              </div>
            </article>
            <article v-else class="capability-card muted">
              <p class="eyebrow">CAMERA</p>
              <span>{{ t("dashboard.cameraUnavailable") }}</span>
            </article>

            <article v-if="capabilities?.fileList" class="capability-card files-card">
              <p class="eyebrow">{{ t("dashboard.files") }}</p>
              <button type="button" :disabled="filesLoading" @click="loadFiles">{{ filesLoading ? t("common.loading") : t("dashboard.loadFiles") }}</button>
              <span v-if="files.length">{{ t("dashboard.fileCount", { count: files.length }) }}</span>
              <ul v-if="files.length" class="file-list">
                <li v-for="file in files.slice(0, 6)" :key="file.devicePath">
                  <span>{{ file.type === "directory" ? "◫" : "·" }}</span>{{ file.name }}
                  <button v-if="file.type === 'file' && capabilities?.jobStart && selectedStatus?.state === 'idle'" type="button" @click="openAction('start', file.devicePath)">{{ t("dashboard.print") }}</button>
                </li>
              </ul>
              <span v-else>{{ t("dashboard.noFiles") }}</span>
            </article>

            <article v-if="capabilities?.temperatureWrite && selectedStatus?.temperatures" class="capability-card temperatures-card">
              <p class="eyebrow">{{ t("dashboard.temperature") }}</p>
              <div v-if="selectedStatus.temperatures.nozzle" class="stepper">
                <span>{{ t("dashboard.nozzle") }} · {{ formatTemperature(selectedStatus.temperatures.nozzle) }}</span>
                <div><button type="button" :disabled="selectedStatus.state !== 'idle'" :aria-label="t('dashboard.decrease')" @click="adjustTemperature('nozzleCelsius', -5)">−5°</button><button type="button" :disabled="selectedStatus.state !== 'idle'" :aria-label="t('dashboard.increase')" @click="adjustTemperature('nozzleCelsius', 5)">+5°</button></div>
              </div>
              <div v-if="selectedStatus.temperatures.bed" class="stepper">
                <span>{{ t("dashboard.bed") }} · {{ formatTemperature(selectedStatus.temperatures.bed) }}</span>
                <div><button type="button" :disabled="selectedStatus.state !== 'idle'" :aria-label="t('dashboard.decrease')" @click="adjustTemperature('bedCelsius', -5)">−5°</button><button type="button" :disabled="selectedStatus.state !== 'idle'" :aria-label="t('dashboard.increase')" @click="adjustTemperature('bedCelsius', 5)">+5°</button></div>
              </div>
            </article>

            <article v-if="capabilities?.fanControl" class="capability-card fan-card">
              <p class="eyebrow">{{ t("dashboard.fan") }}</p>
              <label>
                <input type="range" min="0" max="100" step="1" :disabled="!selectedStatus" :value="selectedStatus?.fans.partCooling ?? 0" @change="queueFan" />
                <span>{{ selectedStatus?.fans.partCooling ?? 0 }}%</span>
              </label>
            </article>

            <article v-if="capabilities?.motionControl && selectedStatus?.state === 'idle'" class="capability-card motion-card">
              <p class="eyebrow">{{ t("dashboard.motion") }}</p>
              <button type="button" @click="openAction('home')">{{ t("dashboard.home") }}</button>
            </article>

            <article v-if="capabilities?.jobStart || capabilities?.jobPause || capabilities?.jobResume || capabilities?.jobCancel" class="capability-card controls-card">
              <p class="eyebrow">{{ t("dashboard.jobs") }}</p>
              <div class="control-actions">
                <button v-if="capabilities.jobPause && selectedStatus?.state === 'printing'" type="button" @click="openAction('pause')">{{ t("dashboard.pause") }}</button>
                <button v-if="capabilities.jobResume && selectedStatus?.state === 'paused'" type="button" @click="openAction('resume')">{{ t("dashboard.resume") }}</button>
                <button v-if="capabilities.jobCancel && ['printing', 'paused'].includes(selectedStatus?.state ?? '')" class="danger" type="button" @click="openAction('cancel')">{{ t("dashboard.cancel") }}</button>
                <span v-if="!['printing', 'paused'].includes(selectedStatus?.state ?? '')">{{ t("dashboard.noJob") }}</span>
              </div>
            </article>

            <article v-if="capabilities?.emergencyStop" class="capability-card emergency-card">
              <p class="eyebrow">M112</p>
              <button class="emergency-button" type="button" @click="emergencyStop">{{ t("dashboard.emergency") }}</button>
            </article>
          </div>
          <p class="cli-note">{{ t("dashboard.cliOnly") }}</p>
        </section>
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
          <p v-if="removalError" class="dialog-error" role="alert"><strong>{{ t("removal.error") }}</strong> {{ removalError }}</p>
          <div class="actions">
            <button type="button" :disabled="removing" @click="closeRemoval">{{ t("common.cancel") }}</button>
            <button class="danger" type="button" :disabled="removing" @click="removePrinter">{{ removing ? t("common.removing") : t("removal.confirm") }}</button>
          </div>
        </DialogPanel>
      </div>
    </Dialog>

    <Dialog :open="actionOpen" @close="!actionSending && (actionOpen = false)" class="dialog">
      <div class="backdrop" aria-hidden="true" />
      <div class="dialog-frame">
        <DialogPanel class="dialog-panel">
          <DialogTitle>{{ t("job.title", { action: pendingAction ?? "" }) }}</DialogTitle>
          <p>{{ t("job.description") }}</p>
          <p v-if="actionError" class="dialog-error" role="alert"><strong>{{ t("job.error") }}</strong> {{ actionError }}</p>
          <div class="actions">
            <button type="button" :disabled="actionSending" @click="actionOpen = false">{{ t("common.cancel") }}</button>
            <button class="danger" type="button" :disabled="actionSending" @click="sendAction">{{ actionSending ? t("common.sending") : t("job.confirm") }}</button>
          </div>
        </DialogPanel>
      </div>
    </Dialog>

    <Dialog :open="diagnosticsOpen" @close="diagnosticsOpen = false" class="dialog">
      <div class="backdrop" aria-hidden="true" />
      <div class="dialog-frame">
        <DialogPanel class="dialog-panel diagnostics-panel">
          <DialogTitle>{{ t("diagnostics.title") }}</DialogTitle>
          <p>{{ t("diagnostics.description") }}</p>
          <p class="diagnostics-redacted">{{ t("diagnostics.redacted") }}</p>
          <dl v-if="diagnostics" class="diagnostic-data">
            <div><dt>VERSION</dt><dd>{{ diagnostics.version }}</dd></div>
            <div><dt>PLATFORM</dt><dd>{{ diagnostics.platform }}</dd></div>
            <div><dt>PROFILES</dt><dd>{{ diagnostics.configuredProfiles }}</dd></div>
            <div><dt>MONITOR</dt><dd>{{ diagnostics.monitorWorkers }} workers / {{ diagnostics.monitorIntervalSeconds }}s</dd></div>
          </dl>
          <p v-else-if="diagnosticsError" class="dialog-error" role="alert">{{ diagnosticsError }}</p>
          <p v-else class="empty">{{ t("diagnostics.noReport") }}</p>
          <button type="button" @click="diagnosticsOpen = false">{{ t("common.close") }}</button>
        </DialogPanel>
      </div>
    </Dialog>
  </main>
</template>
