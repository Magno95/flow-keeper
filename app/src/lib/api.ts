import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";
import { listen as tauriListen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccessResult,
  Check,
  CompanionState,
  ConversationCard,
  EditorStatus,
  Overview,
  ProjectDetail,
  ProjectOption,
  Settings,
  SettingsView,
  SetupState,
  UpdateState,
} from "./types";

// Outside Tauri (browser preview) we use mock data.
const inTauri = isTauri();

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  const { mockInvoke } = await import("./mock");
  return (await mockInvoke(cmd, args)) as T;
}

export function listen<T>(event: string, cb: EventCallback<T>): Promise<UnlistenFn> {
  return inTauri ? tauriListen<T>(event, cb) : Promise.resolve(() => {});
}

export const api = {
  overview: () => invoke<Overview>("get_overview"),
  project: (id: number | null) => invoke<ProjectDetail | null>("get_project", { id }),
  removed: () => invoke<ConversationCard[]>("get_removed"),
  search: (query: string, includeHidden = false) => invoke<ConversationCard[]>("search", { query, includeHidden }),
  companion: () => invoke<CompanionState | null>("get_companion"),
  projects: () => invoke<ProjectOption[]>("list_projects"),
  togglePanel: () => invoke<void>("toggle_panel"),
  hidePanel: () => invoke<void>("hide_panel"),
  open: (id: number) => invoke<AccessResult>("open_conversation", { id }),
  resume: (id: number) => invoke<AccessResult>("resume_conversation", { id }),
  chooseFolderAndResume: (id: number) => invoke<AccessResult | null>("choose_folder_and_resume", { id }),
  hide: (id: number) => invoke<void>("hide_conversation", { id }),
  unhide: (id: number) => invoke<void>("unhide_conversation", { id }),
  dismissAttention: (id: number) => invoke<void>("dismiss_attention", { id }),
  markProjectRead: (id: number | null) => invoke<void>("mark_project_read", { id }),
  rename: (id: number, title: string | null) => invoke<void>("rename_conversation", { id, title }),
  move: (req: { id: number; projectId: number | null; newProjectName: string | null; associateFolder: boolean }) =>
    invoke<number | null>("move_conversation", { req }),
  requestRecap: (id: number) => invoke<void>("request_recap", { id }),
  updateProject: (id: number, name: string | null, description: string | null, setDescription: boolean) =>
    invoke<void>("update_project", { id, name, description, setDescription }),
  setProjectExcluded: (id: number, excluded: boolean) => invoke<void>("set_project_excluded", { id, excluded }),
  mergeProjects: (from: number, into: number) => invoke<void>("merge_projects", { from, into }),
  settings: () => invoke<SettingsView>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  setApiKey: (key: string | null) => invoke<void>("set_api_key", { key }),
  setupState: () => invoke<SetupState>("get_setup_state"),
  refreshEnvironment: () => invoke<SetupState>("refresh_environment"),
  connectClaude: () => invoke<SetupState>("connect_claude"),
  disconnectClaude: () => invoke<SetupState>("disconnect_claude"),
  connectCodex: () => invoke<SetupState>("connect_codex"),
  disconnectCodex: () => invoke<SetupState>("disconnect_codex"),
  doctor: () => invoke<Check[]>("run_doctor"),
  startProbe: () => invoke<string>("start_probe"),
  probeStatus: (nonce: string) => invoke<boolean>("probe_status", { nonce }),
  requestNotifications: () => invoke<boolean>("request_notification_permission"),
  quit: () => invoke<void>("quit_app"),
  editors: () => invoke<EditorStatus[]>("get_editors"),
  installEditorExtension: (bundle: string) => invoke<EditorStatus[]>("install_editor_extension", { bundle }),
  uninstallEditorExtension: (bundle: string) => invoke<EditorStatus[]>("uninstall_editor_extension", { bundle }),
  update: () => invoke<UpdateState>("get_update"),
  checkUpdate: () => invoke<UpdateState>("check_update"),
  dismissUpdate: () => invoke<void>("dismiss_update"),
  installUpdate: () => invoke<void>("install_update"),
  openUpdates: () => invoke<void>("open_updates"),
  setCompanionHeadroom: (extra: number) => invoke<void>("set_companion_headroom", { extra }),
};

export function onChanged(cb: (tick: boolean) => void): Promise<UnlistenFn> {
  return listen<string | null>("tracker://changed", (e) => cb(e.payload === "tick"));
}

export function onCompanion(cb: (s: CompanionState) => void): Promise<UnlistenFn> {
  return listen<CompanionState>("tracker://companion", (e) => cb(e.payload));
}

export function onUpdate(cb: (s: UpdateState) => void): Promise<UnlistenFn> {
  return listen<UpdateState>("tracker://update", (e) => cb(e.payload));
}

export function onPanelOpened(cb: () => void): Promise<UnlistenFn> {
  return listen("tracker://panel-opened", () => cb());
}

export function onPanelClosed(cb: () => void): Promise<UnlistenFn> {
  return listen("tracker://panel-closed", () => cb());
}

export function onNavigate(cb: (to: string) => void): Promise<UnlistenFn> {
  return listen<string>("tracker://navigate", (e) => cb(e.payload));
}
