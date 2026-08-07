export const locales = ["en", "pt-BR"] as const;

export type Locale = (typeof locales)[number];

export type MessageKey =
  | "app.eyebrow"
  | "app.title"
  | "app.status"
  | "app.locale"
  | "app.english"
  | "app.portuguese"
  | "status.connecting"
  | "status.ready"
  | "status.offline"
  | "workspace.eyebrow"
  | "workspace.title"
  | "workspace.description"
  | "workspace.configured"
  | "workspace.printer"
  | "workspace.printers"
  | "workspace.core"
  | "workspace.coreValue"
  | "workspace.gui"
  | "workspace.guiValue"
  | "workspace.monitoring"
  | "profiles.label"
  | "profiles.select"
  | "profiles.remove"
  | "profiles.add"
  | "profiles.save"
  | "profiles.empty"
  | "profiles.loading"
  | "profiles.error"
  | "common.retry"
  | "common.close"
  | "common.cancel"
  | "common.removing"
  | "common.adding"
  | "common.saving"
  | "common.loading"
  | "footer.starting"
  | "footer.private"
  | "drivers.title"
  | "drivers.description"
  | "drivers.empty"
  | "removal.title"
  | "removal.description"
  | "removal.confirm"
  | "removal.error"
  | "addition.title"
  | "addition.description"
  | "addition.name"
  | "addition.driver"
  | "addition.host"
  | "addition.serial"
  | "addition.timeout"
  | "addition.accessCode"
  | "addition.accessCodeKeep"
  | "addition.insecure"
  | "addition.confirm"
  | "addition.error"
  | "addition.discover"
  | "addition.useDiscovery"
  | "addition.discardTitle"
  | "addition.discardDescription"
  | "addition.discardConfirm"
  | "tls.title"
  | "tls.description"
  | "tls.refresh"
  | "tls.confirmTitle"
  | "tls.confirmDescription"
  | "tls.confirm"
  | "dashboard.emptyTitle"
  | "dashboard.emptyDescription"
  | "dashboard.title"
  | "dashboard.refresh"
  | "dashboard.refreshing"
  | "dashboard.state"
  | "dashboard.job"
  | "dashboard.progress"
  | "dashboard.noJob"
  | "dashboard.temperature"
  | "dashboard.cameraUnavailable"
  | "dashboard.cameraPreview"
  | "dashboard.cameraStream"
  | "dashboard.cameraSnapshot"
  | "dashboard.files"
  | "dashboard.loadFiles"
  | "dashboard.noFiles"
  | "dashboard.fileCount"
  | "dashboard.moreFiles"
  | "dashboard.jobs"
  | "dashboard.print"
  | "dashboard.pause"
  | "dashboard.resume"
  | "dashboard.cancel"
  | "dashboard.emergency"
  | "dashboard.actionError"
  | "dashboard.monitoring"
  | "dashboard.monitoringError"
  | "dashboard.cliOnly"
  | "dashboard.nozzle"
  | "dashboard.bed"
  | "dashboard.decrease"
  | "dashboard.increase"
  | "dashboard.fan"
  | "dashboard.motion"
  | "dashboard.home"
  | "job.title"
  | "job.description"
  | "job.confirm"
  | "job.error"
  | "common.sending"
  | "diagnostics.title"
  | "diagnostics.description"
  | "diagnostics.redacted"
  | "diagnostics.noReport"
  | "diagnostics.open"
  | "nav.ariaLabel"
  | "nav.printers"
  | "nav.settings"
  | "nav.files"
  | "nav.selectPrinter"
  | "nav.groupPrinters"
  | "nav.groupSections"
  | "status.idle"
  | "status.onlineIdle"
  | "status.onlineBusy"
  | "status.synchronizing"
  | "status.reconnecting"
  | "status.offlineLabel"
  | "status.errorLabel"
  | "control.printerActions"
  | "control.signalExcellent"
  | "control.signalGood"
  | "control.signalWeak"
  | "control.signalNone"
  | "control.unavailableTitle"
  | "control.unavailableDescription"
  | "control.awaitingTitle"
  | "control.awaitingDescription"
  | "control.presets"
  | "control.presetOff"
  | "control.presetApplied"
  | "control.presetCooling"
  | "control.refreshConnection"
  | "control.reconnecting"
  | "control.faults"
  | "control.faultUnrecoverable"
  | "control.currentJob"
  | "control.complete"
  | "control.preparing"
  | "control.remaining"
  | "control.timeUnavailable"
  | "control.layer"
  | "control.plateOf"
  | "control.cancelRequested"
  | "control.jobPaused"
  | "control.jobResumed"
  | "control.chamber"
  | "control.decreaseTemp"
  | "control.increaseTemp"
  | "control.setTemp"
  | "control.fans"
  | "control.fanPartCooling"
  | "control.fanAuxiliary"
  | "control.fanHeatbreak"
  | "control.fanHotend"
  | "control.fanExhaust"
  | "control.fanChamber"
  | "control.fanMainboard"
  | "control.fanHeat"
  | "control.fanHotendSecondary"
  | "control.fanAuxiliarySecondary"
  | "control.fanPower"
  | "control.lights"
  | "control.lamp"
  | "control.workLight"
  | "control.chamberLight"
  | "control.auxLight"
  | "control.nozzleLeft"
  | "control.nozzleRight"
  | "control.telemetryOnly"
  | "control.stepSize"
  | "control.feedRate"
  | "control.homingStarted"
  | "control.moved"
  | "control.jogUnsupported"
  | "control.jogAxis"
  | "control.fanSet"
  | "control.lightSet"
  | "control.speedProfile"
  | "control.speedSilent"
  | "control.speedStandard"
  | "control.speedSport"
  | "control.speedLudicrous"
  | "control.speedSet"
  | "control.emergencyStopSent"
  | "control.observedAt"
  | "control.observedAtTitle"
  | "camera.title"
  | "camera.live"
  | "camera.snapshotLabel"
  | "camera.loading"
  | "camera.refresh"
  | "camera.snapshot"
  | "camera.maximize"
  | "camera.unavailable"
  | "camera.checkConnection"
  | "camera.refreshFeed"
  | "camera.restored"
  | "camera.snapshotSaved"
  | "materials.title"
  | "materials.externalSpool"
  | "materials.slotExternal"
  | "materials.active"
  | "materials.ready"
  | "materials.humidity"
  | "materials.temperature"
  | "printersView.title"
  | "printersView.description"
  | "printersView.discover"
  | "printersView.totalPrinters"
  | "printersView.online"
  | "printersView.printingNow"
  | "printersView.emptyTitle"
  | "printersView.emptyDescription"
  | "printersView.scan"
  | "printersView.removePrinter"
  | "printersView.removeNamed"
  | "printersView.serialHidden"
  | "printersView.showSerialNamed"
  | "printersView.hideSerialNamed"
  | "printersView.status"
  | "printersView.host"
  | "printersView.discoveredHost"
  | "printersView.useDiscoveredHost"
  | "printersView.editPrinter"
  | "printersView.tlsVerification"
  | "printersView.openControl"
  | "printersView.refreshCertificate"
  | "printersView.refreshCertificateNamed"
  | "printersView.addAnother"
  | "printersView.scanComplete"
  | "printersView.discovered"
  | "printersView.removed"
  | "printersView.tlsRefreshRequested"
  | "printersView.scanning"
  | "settingsView.description"
  | "settingsView.notifications"
  | "settingsView.notifyComplete"
  | "settingsView.notifyCompleteDescription"
  | "settingsView.notifyFailure"
  | "settingsView.notifyFailureDescription"
  | "settingsView.notifyDisconnected"
  | "settingsView.notifyDisconnectedDescription"
  | "settingsView.slicers"
  | "settingsView.noSlicers"
  | "settingsView.slicerName"
  | "settingsView.slicerPath"
  | "settingsView.slicerPathPlaceholder"
  | "settingsView.browse"
  | "settingsView.addSlicer"
  | "settingsView.slicerNameRequired"
  | "settingsView.slicerPathRequired"
  | "settingsView.removeSlicer"
  | "settingsView.removeNamedSlicer"
  | "settingsView.slicerRemoveTitle"
  | "settingsView.slicerRemoveDescription"
  | "settingsView.slicerRemoveConfirm"
  | "settingsView.appearance"
  | "settingsView.theme"
  | "settingsView.themeSystem"
  | "settingsView.themeLight"
  | "settingsView.themeDark"
  | "settingsView.about"
  | "settingsView.license"
  | "settingsView.source"
  | "settingsView.sourceLink"
  | "settingsView.bambuCompatibility"
  | "settingsView.model"
  | "settingsView.authorization"
  | "settingsView.cameraTransport"
  | "settingsView.storageTransport"
  | "settingsView.firmwareModules"
  | "settingsView.quirks"
  | "settingsView.none"
  | "settingsView.version"
  | "settingsView.platform"
  | "settingsView.profiles"
  | "settingsView.monitor"
  | "settingsView.monitorValue"
  | "filesView.title"
  | "filesView.description"
  | "filesView.upload"
  | "filesView.up"
  | "filesView.changeFolder"
  | "filesView.breadcrumb"
  | "filesView.search"
  | "filesView.previewUnavailable"
  | "filesView.filteredBy"
  | "filesView.clearSearch"
  | "filesView.sortBy"
  | "filesView.sortName"
  | "filesView.sortSize"
  | "filesView.sortModified"
  | "filesView.folderModified"
  | "filesView.moreActions"
  | "filesView.openWith"
  | "filesView.deleteTitle"
  | "filesView.deleteDescription"
  | "filesView.deleteConfirm"
  | "filesView.deleted"
  | "filesView.openingIn"
  | "filesView.sentToPrinter"
  | "filesView.downloaded"
  | "filesView.downloadFile"
  | "filesView.downloadingPercent"
  | "filesView.downloadNamed"
  | "filesView.printFile"
  | "filesView.printNamed"
  | "filesView.type"
  | "filesView.size"
  | "filesView.modified"
  | "filesView.name"
  | "filesView.actions"
  | "filesView.emptyTitle"
  | "filesView.emptyDescription"
  | "filesView.uploadDescription"
  | "filesView.uploadFile"
  | "filesView.dragDrop"
  | "filesView.uploadHint"
  | "filesView.printer"
  | "filesView.displayName"
  | "filesView.plate"
  | "filesView.noPrinters"
  | "filesView.printerOffline"
  | "filesView.uploadingTo"
  | "filesView.uploadConfirm"
  | "filesView.uploadedTo"
  | "filesView.uploadedToPrinter"
  | "filesView.notSupported"
  | "common.delete"
  | "common.enabled"
  | "common.disabled"
  | "common.moreActions"
  | "common.closePanel"
  | "confirm.cancelTitle"
  | "confirm.cancelDescription"
  | "confirm.cancelConfirm"
  | "confirm.stopTitle"
  | "confirm.stopDescription"
  | "confirm.stopConfirm"
  | "errors.unknown"
  | "errors.configUnreadable"
  | "errors.configUnwritable"
  | "errors.discoveryFailed"
  | "errors.monitorUnavailable"
  | "errors.profileNotFound"
  | "errors.profileInvalid"
  | "errors.profileExists"
  | "errors.profileMissingName"
  | "errors.profileInvalidName"
  | "errors.profileInvalidHost"
  | "errors.profileInvalidAccessCode"
  | "errors.profileInvalidFingerprint"
  | "errors.profileMissingAccessCode"
  | "errors.accessCodeUnavailable"
  | "errors.keychainFailed"
  | "errors.tlsCredentialsUnavailable"
  | "errors.tlsUnconfirmed"
  | "errors.actionUnconfirmed"
  | "errors.actionUnsupported"
  | "errors.jobFileMissing"
  | "errors.jobFileInvalid"
  | "errors.temperatureTargetMissing"
  | "errors.driverUnsupported"
  | "errors.printerAuthFailed"
  | "errors.printerSigningRequired"
  | "errors.printerAuthorizationConflict"
  | "errors.printerTimeout"
  | "errors.printerOperationFailed"
  | "errors.printerWrongState"
  | "errors.cameraAccessCodeUnavailable"
  | "errors.cameraTlsFailed"
  | "errors.cameraPreviewUnavailable"
  | "errors.preferencesUnreadable"
  | "errors.preferencesUnwritable"
  | "errors.slicerNameMissing"
  | "errors.slicerPathMissing"
  | "errors.slicerAlreadyExists"
  | "errors.slicerNotFound"
  | "errors.thumbnailUnsupported"
  | "errors.thumbnailTooLarge"
  | "errors.thumbnailInvalid"
  | "errors.slicerLaunchFailed"
  | "errors.jogTargetMissing"
  | "errors.fileDestinationUnwritable"
  | "errors.fileIntegrityFailed"
  | "errors.libraryUnavailable"
  | "errors.libraryPathInvalid"
  | "operations.operation"
  | "operations.status"
  | "operations.fileList"
  | "operations.jobStart"
  | "operations.jobPause"
  | "operations.jobResume"
  | "operations.jobCancel"
  | "operations.emergencyStop"
  | "operations.cameraSnapshot"
  | "operations.cameraStream"
  | "operations.temperatureSet"
  | "operations.fanSet"
  | "operations.motionHome"
  | "operations.motionJog"
  | "operations.lightSet"
  | "operations.speedSet"
  | "operations.fileDownload"
  | "operations.fileUpload"
  | "operations.verification"
  | "printerState.idle"
  | "printerState.printing"
  | "printerState.paused"
  | "printerState.error"
  | "printerState.unknown"
  | "status.unknownLabel"
  | "printersView.tlsVerified"
  | "printersView.tlsDisabled"
  | "settingsView.presets"
  | "settingsView.presetsDescription"
  | "settingsView.presetName"
  | "settingsView.presetNozzle"
  | "settingsView.presetBed"
  | "settingsView.addPreset"
  | "settingsView.removePreset"
  | "settingsView.removeNamedPreset"
  | "settingsView.noPresets"
  | "settingsView.restorePresets"
  | "settingsView.presetNameRequired"
  | "settingsView.presetNameTooLong"
  | "settingsView.presetNozzleInvalid"
  | "settingsView.presetBedInvalid"
  | "settingsView.presetRemoveTitle"
  | "settingsView.presetRemoveDescription"
  | "settingsView.presetRemoveConfirm";

type Messages = Record<MessageKey, string>;

const messages: Record<Locale, Messages> = {
  en: {
    "app.eyebrow": "LOCAL-FIRST PRINT CONTROL",
    "app.title": "POLIMERO",
    "app.status": "Application status: {status}",
    "app.locale": "Language",
    "app.english": "English",
    "app.portuguese": "Português (Brasil)",
    "status.connecting": "Connecting",
    "status.ready": "ready",
    "status.offline": "offline",
    "workspace.eyebrow": "RUST + TAURI V2",
    "workspace.title": "Desktop control is coming online.",
    "workspace.description": "The Tauri shell reads your local printer configuration through the shared Rust core.",
    "workspace.configured": "Configured",
    "workspace.printer": "printer",
    "workspace.printers": "printers",
    "workspace.core": "Core",
    "workspace.coreValue": "Shared Rust domain",
    "workspace.gui": "GUI",
    "workspace.guiValue": "Vue desktop surface",
    "workspace.monitoring": "Live local monitoring",
    "profiles.label": "Configured printer profiles",
    "profiles.select": "Open",
    "profiles.remove": "Remove",
    "profiles.add": "Add printer",
    "profiles.save": "Save changes",
    "profiles.empty": "No printers configured yet. Add one with the headless CLI while the setup flow is migrated.",
    "profiles.loading": "Loading local printer profiles…",
    "profiles.error": "We could not load your printer profiles.",
    "common.retry": "Retry",
    "common.close": "Close",
    "common.cancel": "Cancel",
    "common.removing": "Removing…",
    "common.adding": "Verifying…",
    "common.saving": "Saving…",
    "common.loading": "Loading…",
    "footer.starting": "Starting core…",
    "footer.private": "no cloud · no telemetry",
    "drivers.title": "Available printer drivers",
    "drivers.description": "This desktop process invokes the Rust core directly; it does not run a local server.",
    "drivers.empty": "No drivers are registered.",
    "removal.title": "Remove {name}?",
    "removal.description": "This removes the local profile and its stored credentials. The printer itself is not changed.",
    "removal.confirm": "Remove profile",
    "removal.error": "We could not remove this profile.",
    "addition.title": "Add a printer",
    "addition.description": "The printer must respond before its local profile is saved.",
    "addition.name": "Profile name",
    "addition.driver": "Driver",
    "addition.host": "Host or base URL",
    "addition.serial": "Serial number",
    "addition.timeout": "Connection timeout",
    "addition.accessCode": "Access code",
    "addition.accessCodeKeep": "Access code (leave blank to keep existing)",
    "addition.insecure": "Accept an untrusted TLS certificate",
    "addition.confirm": "Verify and save",
    "addition.error": "We could not save this profile.",
    "addition.discover": "Find Bambu printers on this network",
    "addition.useDiscovery": "Use this printer",
    "addition.discardTitle": "Discard printer changes?",
    "addition.discardDescription": "The profile information you entered will be lost.",
    "addition.discardConfirm": "Discard changes",
    "tls.title": "TLS certificate",
    "tls.description": "Capture and review the printer certificate before replacing its local pin.",
    "tls.refresh": "Capture new certificate",
    "tls.confirmTitle": "Replace the TLS pin for {name}?",
    "tls.confirmDescription": "Review this fingerprint before replacing the stored pin. The printer receives no command.",
    "tls.confirm": "Capture and replace pin",
    "dashboard.emptyTitle": "Choose a printer to begin.",
    "dashboard.emptyDescription": "Add a local printer profile, then select it to inspect only the capabilities its driver can safely provide.",
    "dashboard.title": "Live workspace",
    "dashboard.refresh": "Refresh now",
    "dashboard.refreshing": "Refreshing…",
    "dashboard.state": "State",
    "dashboard.job": "Job",
    "dashboard.progress": "Progress",
    "dashboard.noJob": "No active print",
    "dashboard.temperature": "Temperatures",
    "dashboard.cameraUnavailable": "Camera media is unavailable until this driver has a verified camera transport.",
    "dashboard.cameraPreview": "Camera preview is not running.",
    "dashboard.cameraStream": "Start live view",
    "dashboard.cameraSnapshot": "Capture snapshot",
    "dashboard.files": "Printer files",
    "dashboard.loadFiles": "Load files",
    "dashboard.noFiles": "No files reported in the gcodes root.",
    "dashboard.fileCount": "{count} file entries",
    "dashboard.moreFiles": "{count} more on the printer — open Files",
    "dashboard.jobs": "Print controls",
    "dashboard.print": "Print",
    "dashboard.pause": "Pause",
    "dashboard.resume": "Resume",
    "dashboard.cancel": "Cancel print",
    "dashboard.emergency": "Emergency stop",
    "dashboard.actionError": "The printer action was not sent.",
    "dashboard.monitoring": "Configured printer monitor · 5 second local refresh",
    "dashboard.monitoringError": "No live status",
    "dashboard.cliOnly": "File transfer and advanced motion remain available from the CLI until native file selection is integrated.",
    "dashboard.nozzle": "Nozzle",
    "dashboard.bed": "Bed",
    "dashboard.decrease": "Decrease",
    "dashboard.increase": "Increase",
    "dashboard.fan": "Part cooling fan",
    "dashboard.motion": "Motion",
    "dashboard.home": "Home all axes",
    "job.title": "{action} this print?",
    "job.description": "This sends a state-changing command to the selected local printer.",
    "job.confirm": "Confirm action",
    "job.error": "The printer action failed.",
    "common.sending": "Sending…",
    "diagnostics.title": "Redacted diagnostics",
    "diagnostics.description": "This report excludes printer names, hosts, serial numbers, credentials, local paths, and protocol content.",
    "diagnostics.redacted": "Identifiers are redacted by default.",
    "diagnostics.noReport": "No diagnostic report is available.",
    "diagnostics.open": "Diagnostics",
    "nav.ariaLabel": "Workspace navigation",
    "nav.printers": "Manage printers",
    "nav.settings": "Configuration",
    "nav.files": "Files",
    "nav.groupPrinters": "Printers",
    "nav.groupSections": "Sections",
    "nav.selectPrinter": "Select printer",
    "status.idle": "Idle",
    "status.onlineIdle": "Online · idle",
    "status.onlineBusy": "Online · busy",
    "status.synchronizing": "Synchronizing",
    "status.reconnecting": "Reconnecting",
    "status.offlineLabel": "Offline",
    "status.errorLabel": "Error",
    "control.printerActions": "Printer actions",
    "control.signalExcellent": "Excellent signal",
    "control.signalGood": "Good signal",
    "control.signalWeak": "Weak signal",
    "control.signalNone": "No signal",
    "control.unavailableTitle": "Printer unavailable",
    "control.unavailableDescription": "Polimero cannot reach this printer right now. Check the printer connection and try again.",
    "control.awaitingTitle": "Waiting for the first status sample",
    "control.awaitingDescription": "Polimero is establishing the printer heartbeat. Telemetry appears as soon as a sample arrives.",
    "control.presets": "Material presets",
    "control.presetOff": "Cool down",
    "control.presetApplied": "{name} targets requested",
    "control.presetCooling": "Cool down requested",
    "control.refreshConnection": "Refresh connection",
    "control.reconnecting": "Trying to reconnect to printer",
    "control.faults": "Printer alerts",
    "control.faultUnrecoverable": "not recoverable",
    "control.currentJob": "Current job",
    "control.complete": "{percent}% complete",
    "control.preparing": "Preparing — heating and levelling",
    "control.remaining": "{duration} left",
    "control.timeUnavailable": "Time estimate unavailable",
    "control.layer": "Layer {current} / {total}",
    "control.plateOf": "plate {index} of {total}",
    "control.cancelRequested": "Cancel requested. Printer is finishing the current move.",
    "control.jobPaused": "Job paused",
    "control.jobResumed": "Job resumed",
    "control.chamber": "Chamber",
    "control.decreaseTemp": "Decrease {sensor} target temperature",
    "control.increaseTemp": "Increase {sensor} target temperature",
    "control.setTemp": "Target temperature for {sensor}",
    "control.fans": "Fans",
    "control.fanPartCooling": "Parts",
    "control.fanAuxiliary": "Aux",
    "control.fanHeatbreak": "Heatbreak",
    "control.fanHotend": "Hotend",
    "control.fanExhaust": "Exhaust",
    "control.fanChamber": "Chamber",
    "control.fanMainboard": "MC board",
    "control.fanHeat": "Heat",
    "control.fanHotendSecondary": "Hotend 2",
    "control.fanAuxiliarySecondary": "Auxiliary 2",
    "control.fanPower": "{fan} fan power",
    "control.lights": "Lights",
    "control.lamp": "Lamp",
    "control.workLight": "Work light",
    "control.chamberLight": "Chamber light",
    "control.auxLight": "Aux light",
    "control.nozzleLeft": "Left nozzle",
    "control.nozzleRight": "Right nozzle",
    "control.telemetryOnly": "Telemetry only",
    "control.stepSize": "Step size",
    "control.feedRate": "Feed rate",
    "control.homingStarted": "Axes homing started",
    "control.moved": "{axis} moved {distance}",
    "control.jogUnsupported": "Jogging is not supported by this driver",
    "control.jogAxis": "Jog {axis}",
    "control.fanSet": "{fan} fan set to {percent}%",
    "control.lightSet": "{light} {state}",
    "control.speedProfile": "Speed profile",
    "control.speedSilent": "Silent",
    "control.speedStandard": "Standard",
    "control.speedSport": "Sport",
    "control.speedLudicrous": "Ludicrous",
    "control.speedSet": "Speed profile set to {profile}",
    "control.emergencyStopSent": "Emergency stop sent",
    "control.observedAt": "{seconds}s ago",
    "control.observedAtTitle": "Age of the last reading received from the printer",
    "camera.title": "Camera",
    "camera.live": "LIVE",
    "camera.snapshotLabel": "SNAPSHOT",
    "camera.loading": "LOADING…",
    "camera.refresh": "Refresh camera",
    "camera.snapshot": "Save snapshot",
    "camera.maximize": "Maximize camera",
    "camera.unavailable": "Camera unavailable",
    "camera.checkConnection": "Check the printer connection and try again.",
    "camera.refreshFeed": "Refresh feed",
    "camera.restored": "Camera connection restored",
    "camera.snapshotSaved": "Snapshot saved",
    "materials.title": "Filament Spools & Material Systems",
    "materials.externalSpool": "External spool",
    "materials.slotExternal": "EXT",
    "materials.active": "Active",
    "materials.ready": "Ready",
    "materials.humidity": "Humidity",
    "materials.temperature": "Temperature",
    "printersView.title": "Printer management",
    "printersView.description": "Discover and maintain the printers available on your local network.",
    "printersView.discover": "Discover printers",
    "printersView.totalPrinters": "Total printers",
    "printersView.online": "Online",
    "printersView.printingNow": "Printing now",
    "printersView.emptyTitle": "No printers connected",
    "printersView.emptyDescription": "Discover a printer on your local network to start monitoring jobs and materials.",
    "printersView.scan": "Scan local network",
    "printersView.removePrinter": "Remove printer",
    "printersView.removeNamed": "Remove {name}",
    "printersView.serialHidden": "Serial number hidden",
    "printersView.showSerialNamed": "Show serial number for {name}",
    "printersView.hideSerialNamed": "Hide serial number for {name}",
    "printersView.status": "Status",
    "printersView.host": "Host",
    "printersView.discoveredHost": "Discovered host (not applied)",
    "printersView.useDiscoveredHost": "Use this address",
    "printersView.editPrinter": "Edit printer",
    "printersView.tlsVerification": "TLS verification",
    "printersView.openControl": "Open control",
    "printersView.refreshCertificate": "Refresh certificate",
    "printersView.refreshCertificateNamed": "Refresh certificate for {name}",
    "printersView.addAnother": "Add another printer",
    "printersView.scanComplete": "Discovery scan complete",
    "printersView.discovered": "1 printer discovered",
    "printersView.removed": "Printer removed",
    "printersView.tlsRefreshRequested": "TLS certificate refresh requested",
    "printersView.scanning": "Scanning…",
    "settingsView.description": "Choose how Polimero keeps you informed and which tools are available to your team.",
    "settingsView.notifications": "Notifications",
    "settingsView.notifyComplete": "Print complete",
    "settingsView.notifyCompleteDescription": "Show a notification when a print finishes.",
    "settingsView.notifyFailure": "Print failure",
    "settingsView.notifyFailureDescription": "Show a notification when a print fails or is aborted.",
    "settingsView.notifyDisconnected": "Printer disconnected",
    "settingsView.notifyDisconnectedDescription": "Show a notification when the connection is lost.",
    "settingsView.slicers": "Slicer applications",
    "settingsView.noSlicers": "No slicers configured.",
    "settingsView.slicerName": "Name",
    "settingsView.slicerPath": "Executable path",
    "settingsView.slicerPathPlaceholder": "Choose the slicer executable…",
    "settingsView.browse": "Browse…",
    "settingsView.addSlicer": "Add slicer",
    "settingsView.slicerNameRequired": "Enter a slicer name.",
    "settingsView.slicerPathRequired": "Enter the executable path.",
    "settingsView.removeSlicer": "Remove slicer",
    "settingsView.removeNamedSlicer": "Remove {name}",
    "settingsView.slicerRemoveTitle": "Remove {name}?",
    "settingsView.slicerRemoveDescription": "This removes the slicer from Polimero. The application itself is not affected.",
    "settingsView.slicerRemoveConfirm": "Remove slicer",
    "settingsView.presets": "Material presets",
    "settingsView.presetsDescription": "Quick-set targets offered on the control view. Nozzle up to {nozzle} °C, bed up to {bed} °C.",
    "settingsView.presetName": "Material",
    "settingsView.presetNozzle": "Nozzle °C",
    "settingsView.presetBed": "Bed °C",
    "settingsView.addPreset": "Add preset",
    "settingsView.removePreset": "Remove preset",
    "settingsView.removeNamedPreset": "Remove preset {name}",
    "settingsView.noPresets": "No presets configured.",
    "settingsView.restorePresets": "Restore defaults",
    "settingsView.presetNameRequired": "Enter a material name.",
    "settingsView.presetNameTooLong": "Use {max} characters or fewer.",
    "settingsView.presetNozzleInvalid": "Enter a whole number between 0 and {max}.",
    "settingsView.presetBedInvalid": "Enter a whole number between 0 and {max}.",
    "settingsView.presetRemoveTitle": "Remove {name}?",
    "settingsView.presetRemoveDescription": "The preset disappears from the control view. Printer targets already set are unaffected.",
    "settingsView.presetRemoveConfirm": "Remove preset",
    "settingsView.appearance": "Appearance",
    "settingsView.theme": "Theme",
    "settingsView.themeSystem": "System",
    "settingsView.themeLight": "Light",
    "settingsView.themeDark": "Dark",
    "settingsView.about": "About",
    "settingsView.license": "License",
    "settingsView.source": "Source code",
    "settingsView.sourceLink": "View source",
    "settingsView.bambuCompatibility": "Bambu compatibility",
    "settingsView.model": "Model",
    "settingsView.authorization": "Authorization",
    "settingsView.cameraTransport": "Camera transport",
    "settingsView.storageTransport": "Storage transport",
    "settingsView.firmwareModules": "Firmware modules",
    "settingsView.quirks": "Firmware quirks",
    "settingsView.none": "None",
    "settingsView.version": "Version",
    "settingsView.platform": "Platform",
    "settingsView.profiles": "Profiles",
    "settingsView.monitor": "Monitor",
    "settingsView.monitorValue": "{workers} workers / {seconds}s",
    "filesView.title": "File library",
    "filesView.description": "Browse models and directories ready to organize, slice, or print.",
    "filesView.upload": "Upload files",
    "filesView.up": "Up one level",
    "filesView.changeFolder": "Change folder",
    "filesView.breadcrumb": "Breadcrumb",
    "filesView.search": "Search files",
    "filesView.previewUnavailable": "3D preview unavailable",
    "filesView.filteredBy": "Filtered by \u201c{term}\u201d · {count} shown",
    "filesView.clearSearch": "Clear search",
    "filesView.sortBy": "Sort by",
    "filesView.sortName": "Name",
    "filesView.sortSize": "Largest first",
    "filesView.sortModified": "Newest first",
    "filesView.folderModified": "Folder · {date}",
    "filesView.moreActions": "More file actions",
    "filesView.openWith": "Open with {name}",
    "filesView.deleteTitle": "Delete {name}?",
    "filesView.deleteDescription": "This permanently removes the file from printer storage.",
    "filesView.deleteConfirm": "Delete file",
    "filesView.deleted": "{name} deleted",
    "filesView.openingIn": "Opening {name} in Bambu Studio",
    "filesView.sentToPrinter": "{name} sent to printer",
    "filesView.downloaded": "{name} downloaded",
    "filesView.downloadFile": "Download file",
    "filesView.downloadingPercent": "Downloading… {percent}%",
    "filesView.downloadNamed": "Download {name}",
    "filesView.printFile": "Print file",
    "filesView.printNamed": "Print {name}",
    "filesView.type": "Type",
    "filesView.size": "Size",
    "filesView.modified": "Modified",
    "filesView.name": "Name",
    "filesView.actions": "Actions",
    "filesView.emptyTitle": "Nothing here",
    "filesView.emptyDescription": "This directory is empty.",
    "filesView.uploadDescription": "Add new models to your local file library.",
    "filesView.uploadFile": "Upload a file",
    "filesView.dragDrop": "or drag and drop",
    "filesView.uploadHint": "3MF, STL, OBJ up to 200 MB",
    "filesView.printer": "Printer",
    "filesView.displayName": "Job name",
    "filesView.plate": "Plate",
    "filesView.noPrinters": "No printers available",
    "filesView.printerOffline": "{name} — offline",
    "filesView.uploadingTo": "Uploading to {directory}",
    "filesView.uploadConfirm": "Upload",
    "filesView.uploadedTo": "Files uploaded to {directory}",
    "filesView.uploadedToPrinter": "Files uploaded to {name} · {directory}",
    "filesView.notSupported": "File browsing is not supported by this printer.",
    "common.delete": "Delete",
    "common.enabled": "Enabled",
    "common.disabled": "Disabled",
    "common.moreActions": "More actions",
    "common.closePanel": "Close panel",
    "confirm.cancelTitle": "Cancel the print on {name}?",
    "confirm.cancelDescription": "The print stops where it is and cannot be resumed. Filament already used is lost.",
    "confirm.cancelConfirm": "Cancel print",
    "confirm.stopTitle": "Emergency stop on {name}?",
    "confirm.stopDescription": "Motors and heaters are cut immediately. The printer may need a firmware restart before it accepts commands again.",
    "confirm.stopConfirm": "Stop now",
    "errors.unknown": "Something went wrong.",
    "errors.configUnreadable": "Unable to read printer configuration.",
    "errors.configUnwritable": "Unable to save printer configuration.",
    "errors.discoveryFailed": "Printer discovery failed.",
    "errors.monitorUnavailable": "Monitoring is unavailable.",
    "errors.profileNotFound": "Printer profile not found.",
    "errors.profileInvalid": "Invalid printer profile.",
    "errors.profileExists": "A printer profile with this name already exists.",
    "errors.profileMissingName": "Enter a printer name.",
    "errors.profileInvalidName": "That printer name is not valid.",
    "errors.profileInvalidHost": "That host is not valid.",
    "errors.profileInvalidAccessCode": "That access code is not valid.",
    "errors.profileInvalidFingerprint": "That TLS fingerprint is not valid.",
    "errors.profileMissingAccessCode": "This driver requires an access code.",
    "errors.accessCodeUnavailable": "Printer authentication is unavailable.",
    "errors.keychainFailed": "Keychain operation failed.",
    "errors.tlsCredentialsUnavailable": "Printer TLS credentials are unavailable.",
    "errors.tlsUnconfirmed": "Review and confirm the new TLS certificate before replacing the stored fingerprint.",
    "errors.actionUnconfirmed": "Confirm this printer action before sending it.",
    "errors.actionUnsupported": "Unsupported printer action.",
    "errors.jobFileMissing": "Choose a printer file before starting a job.",
    "errors.jobFileInvalid": "This 3MF is not sliced or does not contain a printable plate.",
    "errors.temperatureTargetMissing": "Choose a temperature target first.",
    "errors.driverUnsupported": "This driver does not support {operation}.",
    "errors.printerAuthFailed": "Printer authentication failed.",
    "errors.printerSigningRequired": "The printer requires signed commands for {operation}. Enable Developer Mode/LAN-only mode, or use a signed client.",
    "errors.printerAuthorizationConflict": "Conflicting printer security evidence blocks {operation}. Refresh status and inspect Compatibility diagnostics before retrying.",
    "errors.printerTimeout": "The printer timed out while {operation}.",
    "errors.printerOperationFailed": "The printer failed while {operation}.",
    "errors.printerWrongState": "The printer is {state}; it does not support {operation} right now.",
    "errors.cameraAccessCodeUnavailable": "Camera authentication is unavailable.",
    "errors.cameraTlsFailed": "Camera TLS verification failed.",
    "errors.cameraPreviewUnavailable": "Camera preview is unavailable.",
    "errors.preferencesUnreadable": "Unable to read application preferences.",
    "errors.preferencesUnwritable": "Unable to save application preferences.",
    "errors.slicerNameMissing": "Enter a slicer name.",
    "errors.slicerPathMissing": "Enter a slicer path.",
    "errors.slicerAlreadyExists": "A slicer with this name already exists.",
    "errors.slicerNotFound": "Slicer not found.",
    "errors.thumbnailUnsupported": "This file type cannot be previewed.",
    "errors.thumbnailTooLarge": "This file is too large to preview.",
    "errors.thumbnailInvalid": "This model could not be rendered.",
    "errors.slicerLaunchFailed": "Unable to open the file with that slicer.",
    "errors.jogTargetMissing": "Choose at least one axis to move first.",
    "errors.fileDestinationUnwritable": "Cannot write to the chosen destination.",
    "errors.fileIntegrityFailed": "The downloaded file did not pass its integrity check.",
    "errors.libraryUnavailable": "Unable to access your local file library.",
    "errors.libraryPathInvalid": "That file library path is not valid.",
    "operations.operation": "running this operation",
    "operations.status": "reading status",
    "operations.fileList": "listing files",
    "operations.jobStart": "starting jobs",
    "operations.jobPause": "pausing jobs",
    "operations.jobResume": "resuming jobs",
    "operations.jobCancel": "cancelling jobs",
    "operations.emergencyStop": "stopping the printer",
    "operations.cameraSnapshot": "capturing a snapshot",
    "operations.cameraStream": "streaming the camera",
    "operations.temperatureSet": "setting the temperature",
    "operations.fanSet": "setting the fan speed",
    "operations.motionHome": "homing the printer",
    "operations.motionJog": "jogging the printer",
    "operations.lightSet": "setting the light",
    "operations.speedSet": "setting the speed profile",
    "operations.fileDownload": "downloading the file",
    "operations.fileUpload": "uploading the file",
    "operations.verification": "verifying the profile",
    "printerState.idle": "idle",
    "printerState.printing": "printing",
    "printerState.paused": "paused",
    "printerState.error": "in error",
    "printerState.unknown": "in an unknown state",
    "status.unknownLabel": "Unknown",
    "printersView.tlsVerified": "Verified",
    "printersView.tlsDisabled": "Disabled"
 },
  "pt-BR": {
    "app.eyebrow": "CONTROLE DE IMPRESSÃO LOCAL",
    "app.title": "POLIMERO",
    "app.status": "Status da aplicação: {status}",
    "app.locale": "Idioma",
    "app.english": "English",
    "app.portuguese": "Português (Brasil)",
    "status.connecting": "Conectando",
    "status.ready": "pronto",
    "status.offline": "offline",
    "workspace.eyebrow": "RUST + TAURI V2",
    "workspace.title": "O controle desktop está entrando em operação.",
    "workspace.description": "A interface Tauri lê a configuração local de impressoras pelo núcleo Rust compartilhado.",
    "workspace.configured": "Configuradas",
    "workspace.printer": "impressora",
    "workspace.printers": "impressoras",
    "workspace.core": "Núcleo",
    "workspace.coreValue": "Domínio Rust compartilhado",
    "workspace.gui": "GUI",
    "workspace.guiValue": "Superfície desktop em Vue",
    "workspace.monitoring": "Monitoramento local ao vivo",
    "profiles.label": "Perfis de impressora configurados",
    "profiles.select": "Abrir",
    "profiles.remove": "Remover",
    "profiles.add": "Adicionar impressora",
    "profiles.save": "Salvar alterações",
    "profiles.empty": "Nenhuma impressora foi configurada. Adicione uma pela CLI enquanto o fluxo de configuração é migrado.",
    "profiles.loading": "Carregando perfis de impressora locais…",
    "profiles.error": "Não foi possível carregar seus perfis de impressora.",
    "common.retry": "Tentar novamente",
    "common.close": "Fechar",
    "common.cancel": "Cancelar",
    "common.removing": "Removendo…",
    "common.adding": "Verificando…",
    "common.saving": "Salvando…",
    "common.loading": "Carregando…",
    "footer.starting": "Iniciando núcleo…",
    "footer.private": "sem nuvem · sem telemetria",
    "drivers.title": "Drivers de impressora disponíveis",
    "drivers.description": "Este processo desktop invoca o núcleo Rust diretamente; não há um servidor local.",
    "drivers.empty": "Nenhum driver está registrado.",
    "removal.title": "Remover {name}?",
    "removal.description": "Isso remove o perfil local e suas credenciais armazenadas. A impressora não é alterada.",
    "removal.confirm": "Remover perfil",
    "removal.error": "Não foi possível remover este perfil.",
    "addition.title": "Adicionar uma impressora",
    "addition.description": "A impressora deve responder antes que o perfil local seja salvo.",
    "addition.name": "Nome do perfil",
    "addition.driver": "Driver",
    "addition.host": "Host ou URL base",
    "addition.serial": "Número de série",
    "addition.timeout": "Tempo limite de conexão",
    "addition.accessCode": "Código de acesso",
    "addition.accessCodeKeep": "Código de acesso (deixe em branco para manter o atual)",
    "addition.insecure": "Aceitar um certificado TLS não confiável",
    "addition.confirm": "Verificar e salvar",
    "addition.error": "Não foi possível salvar este perfil.",
    "addition.discover": "Encontrar impressoras Bambu nesta rede",
    "addition.useDiscovery": "Usar esta impressora",
    "addition.discardTitle": "Descartar alterações da impressora?",
    "addition.discardDescription": "As informações de perfil inseridas serão perdidas.",
    "addition.discardConfirm": "Descartar alterações",
    "tls.title": "Certificado TLS",
    "tls.description": "Capture e revise o certificado da impressora antes de substituir a fixação local.",
    "tls.refresh": "Capturar novo certificado",
    "tls.confirmTitle": "Substituir a fixação TLS de {name}?",
    "tls.confirmDescription": "Revise esta impressão digital antes de substituir a fixação armazenada. Nenhum comando é enviado à impressora.",
    "tls.confirm": "Capturar e substituir fixação",
    "dashboard.emptyTitle": "Escolha uma impressora para começar.",
    "dashboard.emptyDescription": "Adicione um perfil local e selecione-o para inspecionar somente os recursos que o driver pode fornecer com segurança.",
    "dashboard.title": "Área ao vivo",
    "dashboard.refresh": "Atualizar agora",
    "dashboard.refreshing": "Atualizando…",
    "dashboard.state": "Estado",
    "dashboard.job": "Trabalho",
    "dashboard.progress": "Progresso",
    "dashboard.noJob": "Nenhuma impressão ativa",
    "dashboard.temperature": "Temperaturas",
    "dashboard.cameraUnavailable": "A mídia da câmera não está disponível até que este driver tenha um transporte de câmera verificado.",
    "dashboard.cameraPreview": "A prévia da câmera não está em execução.",
    "dashboard.cameraStream": "Iniciar visualização ao vivo",
    "dashboard.cameraSnapshot": "Capturar imagem",
    "dashboard.files": "Arquivos da impressora",
    "dashboard.loadFiles": "Carregar arquivos",
    "dashboard.noFiles": "Nenhum arquivo foi informado na raiz gcodes.",
    "dashboard.fileCount": "{count} entradas de arquivo",
    "dashboard.moreFiles": "mais {count} na impressora — abrir Arquivos",
    "dashboard.jobs": "Controles de impressão",
    "dashboard.print": "Imprimir",
    "dashboard.pause": "Pausar",
    "dashboard.resume": "Retomar",
    "dashboard.cancel": "Cancelar impressão",
    "dashboard.emergency": "Parada de emergência",
    "dashboard.actionError": "A ação da impressora não foi enviada.",
    "dashboard.monitoring": "Monitor de impressoras configuradas · atualização local a cada 5 segundos",
    "dashboard.monitoringError": "Sem status ao vivo",
    "dashboard.cliOnly": "Transferência de arquivos e movimento avançado continuam disponíveis na CLI até a integração da seleção nativa de arquivos.",
    "dashboard.nozzle": "Bico",
    "dashboard.bed": "Mesa",
    "dashboard.decrease": "Diminuir",
    "dashboard.increase": "Aumentar",
    "dashboard.fan": "Ventilação da peça",
    "dashboard.motion": "Movimento",
    "dashboard.home": "Referenciar todos os eixos",
    "job.title": "{action} esta impressão?",
    "job.description": "Isso envia um comando que altera o estado para a impressora local selecionada.",
    "job.confirm": "Confirmar ação",
    "job.error": "A ação da impressora falhou.",
    "common.sending": "Enviando…",
    "diagnostics.title": "Diagnóstico com dados ocultos",
    "diagnostics.description": "Este relatório exclui nomes de impressora, hosts, números de série, credenciais, caminhos locais e conteúdo de protocolo.",
    "diagnostics.redacted": "Identificadores são ocultados por padrão.",
    "diagnostics.noReport": "Nenhum relatório de diagnóstico está disponível.",
    "diagnostics.open": "Diagnóstico",
    "nav.ariaLabel": "Navegação do espaço de trabalho",
    "nav.printers": "Gerenciar impressoras",
    "nav.settings": "Configuração",
    "nav.files": "Arquivos",
    "nav.groupPrinters": "Impressoras",
    "nav.groupSections": "Seções",
    "nav.selectPrinter": "Selecionar impressora",
    "status.idle": "Ociosa",
    "status.onlineIdle": "Online · ociosa",
    "status.onlineBusy": "Online · ocupada",
    "status.synchronizing": "Sincronizando",
    "status.reconnecting": "Reconectando",
    "status.offlineLabel": "Offline",
    "status.errorLabel": "Erro",
    "control.printerActions": "Ações da impressora",
    "control.signalExcellent": "Sinal excelente",
    "control.signalGood": "Sinal bom",
    "control.signalWeak": "Sinal fraco",
    "control.signalNone": "Sem sinal",
    "control.unavailableTitle": "Impressora indisponível",
    "control.unavailableDescription": "O Polimero não consegue alcançar esta impressora no momento. Verifique a conexão da impressora e tente novamente.",
    "control.awaitingTitle": "Aguardando a primeira amostra de status",
    "control.awaitingDescription": "O Polimero está estabelecendo o heartbeat da impressora. A telemetria aparece assim que uma amostra chegar.",
    "control.presets": "Predefinições de material",
    "control.presetOff": "Resfriar",
    "control.presetApplied": "Alvos de {name} solicitados",
    "control.presetCooling": "Resfriamento solicitado",
    "control.refreshConnection": "Atualizar conexão",
    "control.reconnecting": "Tentando reconectar à impressora",
    "control.faults": "Alertas da impressora",
    "control.faultUnrecoverable": "não recuperável",
    "control.currentJob": "Trabalho atual",
    "control.complete": "{percent}% concluído",
    "control.preparing": "Preparando — aquecendo e nivelando",
    "control.remaining": "faltam {duration}",
    "control.timeUnavailable": "Estimativa de tempo indisponível",
    "control.layer": "Camada {current} / {total}",
    "control.plateOf": "placa {index} de {total}",
    "control.cancelRequested": "Cancelamento solicitado. A impressora está concluindo o movimento atual.",
    "control.jobPaused": "Trabalho pausado",
    "control.jobResumed": "Trabalho retomado",
    "control.chamber": "Câmara",
    "control.decreaseTemp": "Diminuir a temperatura alvo de {sensor}",
    "control.increaseTemp": "Aumentar a temperatura alvo de {sensor}",
    "control.setTemp": "Temperatura alvo para {sensor}",
    "control.fans": "Ventoinhas",
    "control.fanPartCooling": "Peças",
    "control.fanAuxiliary": "Aux",
    "control.fanHeatbreak": "Heatbreak",
    "control.fanHotend": "Hotend",
    "control.fanExhaust": "Exaustão",
    "control.fanChamber": "Câmara",
    "control.fanMainboard": "Placa controladora",
    "control.fanHeat": "Aquecimento",
    "control.fanHotendSecondary": "Hotend 2",
    "control.fanAuxiliarySecondary": "Auxiliar 2",
    "control.fanPower": "Potência da ventoinha: {fan}",
    "control.lights": "Luzes",
    "control.lamp": "Lâmpada",
    "control.workLight": "Luz de trabalho",
    "control.chamberLight": "Luz da câmara",
    "control.auxLight": "Luz auxiliar",
    "control.nozzleLeft": "Bico esquerdo",
    "control.nozzleRight": "Bico direito",
    "control.telemetryOnly": "Somente telemetria",
    "control.stepSize": "Tamanho do passo",
    "control.feedRate": "Velocidade de avanço",
    "control.homingStarted": "Referenciamento dos eixos iniciado",
    "control.moved": "{axis} movido {distance}",
    "control.jogUnsupported": "Este driver não suporta movimento manual",
    "control.jogAxis": "Mover {axis}",
    "control.fanSet": "Ventoinha {fan} ajustada para {percent}%",
    "control.lightSet": "{light} {state}",
    "control.speedProfile": "Perfil de velocidade",
    "control.speedSilent": "Silencioso",
    "control.speedStandard": "Padrão",
    "control.speedSport": "Esportivo",
    "control.speedLudicrous": "Absurdo",
    "control.speedSet": "Perfil de velocidade definido como {profile}",
    "control.emergencyStopSent": "Parada de emergência enviada",
    "control.observedAt": "há {seconds}s",
    "control.observedAtTitle": "Idade da última leitura recebida da impressora",
    "camera.title": "Câmera",
    "camera.live": "AO VIVO",
    "camera.snapshotLabel": "CAPTURA",
    "camera.loading": "CARREGANDO…",
    "camera.refresh": "Atualizar câmera",
    "camera.snapshot": "Salvar captura",
    "camera.maximize": "Maximizar câmera",
    "camera.unavailable": "Câmera indisponível",
    "camera.checkConnection": "Verifique a conexão da impressora e tente novamente.",
    "camera.refreshFeed": "Atualizar transmissão",
    "camera.restored": "Conexão da câmera restabelecida",
    "camera.snapshotSaved": "Captura salva",
    "materials.title": "Carretéis de filamento e sistemas de material",
    "materials.externalSpool": "Carretel externo",
    "materials.slotExternal": "EXT",
    "materials.active": "Ativo",
    "materials.ready": "Pronto",
    "materials.humidity": "Umidade",
    "materials.temperature": "Temperatura",
    "printersView.title": "Gerenciamento de impressoras",
    "printersView.description": "Descubra e mantenha as impressoras disponíveis na sua rede local.",
    "printersView.discover": "Descobrir impressoras",
    "printersView.totalPrinters": "Total de impressoras",
    "printersView.online": "Online",
    "printersView.printingNow": "Imprimindo agora",
    "printersView.emptyTitle": "Nenhuma impressora conectada",
    "printersView.emptyDescription": "Descubra uma impressora na sua rede local para começar a monitorar trabalhos e materiais.",
    "printersView.scan": "Buscar na rede local",
    "printersView.removePrinter": "Remover impressora",
    "printersView.removeNamed": "Remover {name}",
    "printersView.serialHidden": "Número de série oculto",
    "printersView.showSerialNamed": "Mostrar número de série de {name}",
    "printersView.hideSerialNamed": "Ocultar número de série de {name}",
    "printersView.status": "Status",
    "printersView.host": "Host",
    "printersView.discoveredHost": "Host descoberto (não aplicado)",
    "printersView.useDiscoveredHost": "Usar este endereço",
    "printersView.editPrinter": "Editar impressora",
    "printersView.tlsVerification": "Verificação TLS",
    "printersView.openControl": "Abrir controle",
    "printersView.refreshCertificate": "Atualizar certificado",
    "printersView.refreshCertificateNamed": "Atualizar certificado de {name}",
    "printersView.addAnother": "Adicionar outra impressora",
    "printersView.scanComplete": "Busca de descoberta concluída",
    "printersView.discovered": "1 impressora descoberta",
    "printersView.removed": "Impressora removida",
    "printersView.tlsRefreshRequested": "Atualização do certificado TLS solicitada",
    "printersView.scanning": "Buscando…",
    "settingsView.description": "Escolha como o Polimero mantém você informado e quais ferramentas estão disponíveis para a sua equipe.",
    "settingsView.notifications": "Notificações",
    "settingsView.notifyComplete": "Impressão concluída",
    "settingsView.notifyCompleteDescription": "Mostrar uma notificação quando uma impressão terminar.",
    "settingsView.notifyFailure": "Falha de impressão",
    "settingsView.notifyFailureDescription": "Mostrar uma notificação quando uma impressão falhar ou for cancelada.",
    "settingsView.notifyDisconnected": "Impressora desconectada",
    "settingsView.notifyDisconnectedDescription": "Mostrar uma notificação quando a conexão for perdida.",
    "settingsView.slicers": "Aplicativos de fatiamento",
    "settingsView.noSlicers": "Nenhum fatiador configurado.",
    "settingsView.slicerName": "Nome",
    "settingsView.slicerPath": "Caminho do executável",
    "settingsView.slicerPathPlaceholder": "Escolha o executável do fatiador…",
    "settingsView.browse": "Procurar…",
    "settingsView.addSlicer": "Adicionar fatiador",
    "settingsView.slicerNameRequired": "Informe um nome para o fatiador.",
    "settingsView.slicerPathRequired": "Informe o caminho do executável.",
    "settingsView.removeSlicer": "Remover fatiador",
    "settingsView.removeNamedSlicer": "Remover {name}",
    "settingsView.slicerRemoveTitle": "Remover {name}?",
    "settingsView.slicerRemoveDescription": "Isso remove o fatiador do Polimero. O aplicativo em si não é afetado.",
    "settingsView.slicerRemoveConfirm": "Remover fatiador",
    "settingsView.presets": "Predefinições de material",
    "settingsView.presetsDescription": "Alvos rápidos oferecidos na visão de controle. Bico até {nozzle} °C, mesa até {bed} °C.",
    "settingsView.presetName": "Material",
    "settingsView.presetNozzle": "Bico °C",
    "settingsView.presetBed": "Mesa °C",
    "settingsView.addPreset": "Adicionar predefinição",
    "settingsView.removePreset": "Remover predefinição",
    "settingsView.removeNamedPreset": "Remover predefinição {name}",
    "settingsView.noPresets": "Nenhuma predefinição configurada.",
    "settingsView.restorePresets": "Restaurar padrões",
    "settingsView.presetNameRequired": "Informe um nome de material.",
    "settingsView.presetNameTooLong": "Use no máximo {max} caracteres.",
    "settingsView.presetNozzleInvalid": "Informe um número inteiro entre 0 e {max}.",
    "settingsView.presetBedInvalid": "Informe um número inteiro entre 0 e {max}.",
    "settingsView.presetRemoveTitle": "Remover {name}?",
    "settingsView.presetRemoveDescription": "A predefinição some da visão de controle. Alvos já definidos na impressora não são afetados.",
    "settingsView.presetRemoveConfirm": "Remover predefinição",
    "settingsView.appearance": "Aparência",
    "settingsView.theme": "Tema",
    "settingsView.themeSystem": "Sistema",
    "settingsView.themeLight": "Claro",
    "settingsView.themeDark": "Escuro",
    "settingsView.about": "Sobre",
    "settingsView.license": "Licença",
    "settingsView.source": "Código-fonte",
    "settingsView.sourceLink": "Ver código-fonte",
    "settingsView.bambuCompatibility": "Compatibilidade Bambu",
    "settingsView.model": "Modelo",
    "settingsView.authorization": "Autorização",
    "settingsView.cameraTransport": "Transporte da câmera",
    "settingsView.storageTransport": "Transporte de armazenamento",
    "settingsView.firmwareModules": "Módulos de firmware",
    "settingsView.quirks": "Ajustes de firmware",
    "settingsView.none": "Nenhum",
    "settingsView.version": "Versão",
    "settingsView.platform": "Plataforma",
    "settingsView.profiles": "Perfis",
    "settingsView.monitor": "Monitor",
    "settingsView.monitorValue": "{workers} workers / {seconds}s",
    "filesView.title": "Biblioteca de arquivos",
    "filesView.description": "Navegue por modelos e diretórios prontos para organizar, fatiar ou imprimir.",
    "filesView.upload": "Enviar arquivos",
    "filesView.up": "Subir um nível",
    "filesView.changeFolder": "Trocar pasta",
    "filesView.breadcrumb": "Trilha de navegação",
    "filesView.search": "Buscar arquivos",
    "filesView.previewUnavailable": "Pré-visualização 3D indisponível",
    "filesView.filteredBy": "Filtrado por \u201c{term}\u201d · {count} exibidos",
    "filesView.clearSearch": "Limpar busca",
    "filesView.sortBy": "Ordenar por",
    "filesView.sortName": "Nome",
    "filesView.sortSize": "Maiores primeiro",
    "filesView.sortModified": "Mais recentes primeiro",
    "filesView.folderModified": "Pasta · {date}",
    "filesView.moreActions": "Mais ações do arquivo",
    "filesView.openWith": "Abrir com {name}",
    "filesView.deleteTitle": "Excluir {name}?",
    "filesView.deleteDescription": "Isso remove permanentemente o arquivo do armazenamento da impressora.",
    "filesView.deleteConfirm": "Excluir arquivo",
    "filesView.deleted": "{name} excluído",
    "filesView.openingIn": "Abrindo {name} no Bambu Studio",
    "filesView.sentToPrinter": "{name} enviado para a impressora",
    "filesView.downloaded": "{name} baixado",
    "filesView.downloadFile": "Baixar arquivo",
    "filesView.downloadingPercent": "Baixando… {percent}%",
    "filesView.downloadNamed": "Baixar {name}",
    "filesView.printFile": "Imprimir arquivo",
    "filesView.printNamed": "Imprimir {name}",
    "filesView.type": "Tipo",
    "filesView.size": "Tamanho",
    "filesView.modified": "Modificado",
    "filesView.name": "Nome",
    "filesView.actions": "Ações",
    "filesView.emptyTitle": "Nada por aqui",
    "filesView.emptyDescription": "Este diretório está vazio.",
    "filesView.uploadDescription": "Adicione novos modelos à sua biblioteca de arquivos local.",
    "filesView.uploadFile": "Enviar um arquivo",
    "filesView.dragDrop": "ou arraste e solte",
    "filesView.uploadHint": "3MF, STL, OBJ de até 200 MB",
    "filesView.printer": "Impressora",
    "filesView.displayName": "Nome do trabalho",
    "filesView.plate": "Mesa",
    "filesView.noPrinters": "Nenhuma impressora disponível",
    "filesView.printerOffline": "{name} — offline",
    "filesView.uploadingTo": "Enviando para {directory}",
    "filesView.uploadConfirm": "Enviar",
    "filesView.uploadedTo": "Arquivos enviados para {directory}",
    "filesView.uploadedToPrinter": "Arquivos enviados para {name} · {directory}",
    "filesView.notSupported": "A navegação de arquivos não é suportada por esta impressora.",
    "common.delete": "Excluir",
    "common.enabled": "Ativada",
    "common.disabled": "Desativada",
    "common.moreActions": "Mais ações",
    "common.closePanel": "Fechar painel",
    "confirm.cancelTitle": "Cancelar a impressão em {name}?",
    "confirm.cancelDescription": "A impressão para onde está e não pode ser retomada. O filamento já usado é perdido.",
    "confirm.cancelConfirm": "Cancelar impressão",
    "confirm.stopTitle": "Parada de emergência em {name}?",
    "confirm.stopDescription": "Os motores e aquecedores são cortados imediatamente. A impressora pode precisar reiniciar o firmware antes de aceitar comandos novamente.",
    "confirm.stopConfirm": "Parar agora",
    "errors.unknown": "Algo deu errado.",
    "errors.configUnreadable": "Não foi possível ler a configuração da impressora.",
    "errors.configUnwritable": "Não foi possível salvar a configuração da impressora.",
    "errors.discoveryFailed": "A busca por impressoras falhou.",
    "errors.monitorUnavailable": "O monitoramento está indisponível.",
    "errors.profileNotFound": "Perfil de impressora não encontrado.",
    "errors.profileInvalid": "Perfil de impressora inválido.",
    "errors.profileExists": "Já existe um perfil de impressora com este nome.",
    "errors.profileMissingName": "Informe um nome para a impressora.",
    "errors.profileInvalidName": "Esse nome de impressora não é válido.",
    "errors.profileInvalidHost": "Esse host não é válido.",
    "errors.profileInvalidAccessCode": "Esse código de acesso não é válido.",
    "errors.profileInvalidFingerprint": "Essa impressão digital TLS não é válida.",
    "errors.profileMissingAccessCode": "Este driver exige um código de acesso.",
    "errors.accessCodeUnavailable": "A autenticação da impressora está indisponível.",
    "errors.keychainFailed": "A operação no chaveiro falhou.",
    "errors.tlsCredentialsUnavailable": "As credenciais TLS da impressora estão indisponíveis.",
    "errors.tlsUnconfirmed": "Revise e confirme o novo certificado TLS antes de substituir a impressão digital armazenada.",
    "errors.actionUnconfirmed": "Confirme esta ação antes de enviá-la à impressora.",
    "errors.actionUnsupported": "Ação de impressora não suportada.",
    "errors.jobFileMissing": "Escolha um arquivo antes de iniciar uma impressão.",
    "errors.jobFileInvalid": "Este 3MF não está fatiado ou não contém uma mesa imprimível.",
    "errors.temperatureTargetMissing": "Escolha primeiro uma temperatura alvo.",
    "errors.driverUnsupported": "Este driver não suporta {operation}.",
    "errors.printerAuthFailed": "A autenticação da impressora falhou.",
    "errors.printerSigningRequired": "A impressora exige comandos assinados para {operation}. Ative o Modo de Desenvolvedor/modo somente LAN ou use um cliente com assinatura.",
    "errors.printerAuthorizationConflict": "Evidências conflitantes de segurança bloqueiam {operation}. Atualize o status e verifique os diagnósticos de Compatibilidade antes de tentar novamente.",
    "errors.printerTimeout": "A impressora expirou durante {operation}.",
    "errors.printerOperationFailed": "A impressora falhou durante {operation}.",
    "errors.printerWrongState": "A impressora está {state} e não suporta {operation} agora.",
    "errors.cameraAccessCodeUnavailable": "A autenticação da câmera está indisponível.",
    "errors.cameraTlsFailed": "A verificação TLS da câmera falhou.",
    "errors.cameraPreviewUnavailable": "A pré-visualização da câmera está indisponível.",
    "errors.preferencesUnreadable": "Não foi possível ler as preferências do aplicativo.",
    "errors.preferencesUnwritable": "Não foi possível salvar as preferências do aplicativo.",
    "errors.slicerNameMissing": "Informe o nome do fatiador.",
    "errors.slicerPathMissing": "Informe o caminho do fatiador.",
    "errors.slicerAlreadyExists": "Já existe um fatiador com este nome.",
    "errors.slicerNotFound": "Fatiador não encontrado.",
    "errors.thumbnailUnsupported": "Este tipo de arquivo não pode ser pré-visualizado.",
    "errors.thumbnailTooLarge": "Este arquivo é grande demais para pré-visualizar.",
    "errors.thumbnailInvalid": "Não foi possível renderizar este modelo.",
    "errors.slicerLaunchFailed": "Não foi possível abrir o arquivo com este fatiador.",
    "errors.jogTargetMissing": "Escolha ao menos um eixo para mover.",
    "errors.fileDestinationUnwritable": "Não é possível gravar no destino escolhido.",
    "errors.fileIntegrityFailed": "O arquivo baixado não passou na verificação de integridade.",
    "errors.libraryUnavailable": "Não foi possível acessar sua biblioteca de arquivos local.",
    "errors.libraryPathInvalid": "Este caminho da biblioteca de arquivos não é válido.",
    "operations.operation": "esta operação",
    "operations.status": "o status",
    "operations.fileList": "a listagem de arquivos",
    "operations.jobStart": "o início de impressões",
    "operations.jobPause": "a pausa de impressões",
    "operations.jobResume": "a retomada de impressões",
    "operations.jobCancel": "o cancelamento de impressões",
    "operations.emergencyStop": "a parada de emergência",
    "operations.cameraSnapshot": "as fotos da câmera",
    "operations.cameraStream": "a transmissão da câmera",
    "operations.temperatureSet": "o controle de temperatura",
    "operations.fanSet": "o controle de ventoinhas",
    "operations.motionHome": "o controle de movimento",
    "operations.motionJog": "o movimento manual",
    "operations.lightSet": "o controle de iluminação",
    "operations.speedSet": "o controle de velocidade",
    "operations.fileDownload": "o download do arquivo",
    "operations.fileUpload": "o envio do arquivo",
    "operations.verification": "a verificação do perfil",
    "printerState.idle": "ociosa",
    "printerState.printing": "imprimindo",
    "printerState.paused": "pausada",
    "printerState.error": "com erro",
    "printerState.unknown": "em estado desconhecido",
    "status.unknownLabel": "Desconhecido",
    "printersView.tlsVerified": "Verificado",
    "printersView.tlsDisabled": "Desativado"
 }
};

export function preferredLocale(language = navigator.language): Locale {
  return language.toLowerCase().startsWith("pt") ? "pt-BR" : "en";
}

export function translate(locale: Locale, key: MessageKey, values: Record<string, string | number> = {}): string {
  // Codes can arrive from the backend, so an unknown key degrades to the code
  // itself rather than throwing inside a render.
  const template = messages[locale][key] ?? messages.en[key] ?? key;
  return template.replace(/\{(\w+)\}/g, (_, name: string) => String(values[name] ?? `{${name}}`));
}
