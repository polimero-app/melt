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
  | "profiles.label"
  | "profiles.remove"
  | "profiles.empty"
  | "profiles.loading"
  | "profiles.error"
  | "common.retry"
  | "common.close"
  | "common.cancel"
  | "common.removing"
  | "footer.starting"
  | "footer.private"
  | "drivers.title"
  | "drivers.description"
  | "drivers.empty"
  | "removal.title"
  | "removal.description"
  | "removal.confirm"
  | "removal.error";

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
    "profiles.label": "Configured printer profiles",
    "profiles.remove": "Remove",
    "profiles.empty": "No printers configured yet. Add one with the headless CLI while the setup flow is migrated.",
    "profiles.loading": "Loading local printer profiles…",
    "profiles.error": "We could not load your printer profiles.",
    "common.retry": "Retry",
    "common.close": "Close",
    "common.cancel": "Cancel",
    "common.removing": "Removing…",
    "footer.starting": "Starting core…",
    "footer.private": "no cloud · no telemetry",
    "drivers.title": "Available printer drivers",
    "drivers.description": "This desktop process invokes the Rust core directly; it does not run a local server.",
    "drivers.empty": "No drivers are registered.",
    "removal.title": "Remove {name}?",
    "removal.description": "This removes the local profile and its stored credentials. The printer itself is not changed.",
    "removal.confirm": "Remove profile",
    "removal.error": "We could not remove this profile."
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
    "profiles.label": "Perfis de impressora configurados",
    "profiles.remove": "Remover",
    "profiles.empty": "Nenhuma impressora foi configurada. Adicione uma pela CLI enquanto o fluxo de configuração é migrado.",
    "profiles.loading": "Carregando perfis de impressora locais…",
    "profiles.error": "Não foi possível carregar seus perfis de impressora.",
    "common.retry": "Tentar novamente",
    "common.close": "Fechar",
    "common.cancel": "Cancelar",
    "common.removing": "Removendo…",
    "footer.starting": "Iniciando núcleo…",
    "footer.private": "sem nuvem · sem telemetria",
    "drivers.title": "Drivers de impressora disponíveis",
    "drivers.description": "Este processo desktop invoca o núcleo Rust diretamente; não há um servidor local.",
    "drivers.empty": "Nenhum driver está registrado.",
    "removal.title": "Remover {name}?",
    "removal.description": "Isso remove o perfil local e suas credenciais armazenadas. A impressora não é alterada.",
    "removal.confirm": "Remover perfil",
    "removal.error": "Não foi possível remover este perfil."
  }
};

export function preferredLocale(language = navigator.language): Locale {
  return language.toLowerCase().startsWith("pt") ? "pt-BR" : "en";
}

export function translate(locale: Locale, key: MessageKey, values: Record<string, string | number> = {}): string {
  return messages[locale][key].replace(/\{(\w+)\}/g, (_, name: string) => String(values[name] ?? `{${name}}`));
}
