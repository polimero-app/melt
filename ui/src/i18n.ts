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
  | "profiles.empty"
  | "profiles.loading"
  | "profiles.error"
  | "common.retry"
  | "common.close"
  | "common.cancel"
  | "common.removing"
  | "common.adding"
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
  | "addition.insecure"
  | "addition.confirm"
  | "addition.error"
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
  | "dashboard.files"
  | "dashboard.loadFiles"
  | "dashboard.noFiles"
  | "dashboard.fileCount"
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
  | "diagnostics.open";

type Messages = Record<MessageKey, string>;

const messages: Record<Locale, Messages> = {
  en: {
    "app.eyebrow": "LOCAL-FIRST PRINT CONTROL",
    "app.title": "POLIMERO",
    "app.status": "Application status: {status}",
    "app.locale": "Language",
    "app.english": "English",
    "app.portuguese": "Português (Brasil)",
    "status.connecting": "connecting",
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
    "profiles.empty": "No printers configured yet. Add one with the headless CLI while the setup flow is migrated.",
    "profiles.loading": "Loading local printer profiles…",
    "profiles.error": "We could not load your printer profiles.",
    "common.retry": "Retry",
    "common.close": "Close",
    "common.cancel": "Cancel",
    "common.removing": "Removing…",
    "common.adding": "Verifying…",
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
    "addition.insecure": "Accept an untrusted TLS certificate",
    "addition.confirm": "Verify and save",
    "addition.error": "We could not save this profile.",
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
    "dashboard.files": "Printer files",
    "dashboard.loadFiles": "Load files",
    "dashboard.noFiles": "No files reported in the gcodes root.",
    "dashboard.fileCount": "{count} file entries",
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
    "diagnostics.open": "Diagnostics"
  },
  "pt-BR": {
    "app.eyebrow": "CONTROLE DE IMPRESSÃO LOCAL",
    "app.title": "POLIMERO",
    "app.status": "Status da aplicação: {status}",
    "app.locale": "Idioma",
    "app.english": "English",
    "app.portuguese": "Português (Brasil)",
    "status.connecting": "conectando",
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
    "profiles.empty": "Nenhuma impressora foi configurada. Adicione uma pela CLI enquanto o fluxo de configuração é migrado.",
    "profiles.loading": "Carregando perfis de impressora locais…",
    "profiles.error": "Não foi possível carregar seus perfis de impressora.",
    "common.retry": "Tentar novamente",
    "common.close": "Fechar",
    "common.cancel": "Cancelar",
    "common.removing": "Removendo…",
    "common.adding": "Verificando…",
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
    "addition.insecure": "Aceitar um certificado TLS não confiável",
    "addition.confirm": "Verificar e salvar",
    "addition.error": "Não foi possível salvar este perfil.",
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
    "dashboard.files": "Arquivos da impressora",
    "dashboard.loadFiles": "Carregar arquivos",
    "dashboard.noFiles": "Nenhum arquivo foi informado na raiz gcodes.",
    "dashboard.fileCount": "{count} entradas de arquivo",
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
    "diagnostics.open": "Diagnóstico"
  }
};

export function preferredLocale(language = navigator.language): Locale {
  return language.toLowerCase().startsWith("pt") ? "pt-BR" : "en";
}

export function translate(locale: Locale, key: MessageKey, values: Record<string, string | number> = {}): string {
  return messages[locale][key].replace(/\{(\w+)\}/g, (_, name: string) => String(values[name] ?? `{${name}}`));
}
