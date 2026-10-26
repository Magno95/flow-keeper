export type DisplayState = "working" | "waiting" | "finished" | "idle" | "closed" | "unknown";
export type SummaryStatus = "waiting" | "pending" | "ready" | "stale" | "retry" | "failed" | "disabled" | "excluded" | "budget";
export type Mood = "rest" | "working" | "attention" | "problem";

export interface ConversationCard {
  id: number;
  projectId: number | null;
  title: string;
  titleIsProvisional: boolean;
  summary: string | null;
  summaryStatus: SummaryStatus;
  summaryAt: number | null;
  state: DisplayState;
  stateLabel: string;
  stateReason: string | null;
  agent: string;
  environment: string;
  environmentIsGuess: boolean;
  folderLabel: string | null;
  lastActivityAt: number;
  hidden: boolean;
  attentionSeen: boolean;
  /** The turn finished and you haven't looked at it yet. */
  unread: boolean;
  canFocus: boolean;
  canResume: boolean;
}

export interface Counts {
  total: number;
  open: number;
  closed: number;
  working: number;
  waiting: number;
  finished: number;
  unread: number;
}

export interface ProjectCard {
  id: number | null;
  name: string;
  description: string | null;
  recapExcluded: boolean;
  counts: Counts;
  lastActivityAt: number;
}

export interface Overview {
  projects: ProjectCard[];
  waiting: ConversationCard[];
  hiddenCount: number;
  totalConversations: number;
}

export interface ProjectDetail {
  project: ProjectCard;
  conversations: ConversationCard[];
}

export interface ProjectOption {
  id: number;
  name: string;
}

export interface CompanionState {
  mood: Mood;
  waiting: number;
  working: number;
}

export type AccessResult =
  | { outcome: "focused"; app: string }
  | { outcome: "appActivated"; app: string; note: string }
  | { outcome: "resumed"; app: string }
  | { outcome: "unreachable"; message: string; command: string | null }
  | { outcome: "needsFolder"; message: string };

export type RecapProviderKind = "claude_cli" | "api" | "off";

export interface Settings {
  recapProvider: RecapProviderKind;
  recapCliModel: string;
  recapApiModel: string;
  recapDailyLimit: number;
  recapMonthlyBudgetUsd: number;
  notifyAttention: boolean;
  notifyFinished: boolean;
  doNotDisturb: boolean;
  trackPrintSessions: boolean;
  shortcut: string;
  companionVisible: boolean;
  launchAtLogin: boolean;
  onboardingDone: boolean;
}

export interface SettingsView {
  settings: Settings;
  apiKeyPresent: boolean;
}

export interface SetupState {
  detected: boolean;
  claudeFound: boolean;
  claudeVersion: string | null;
  versionTested: boolean;
  pluginInstalled: boolean;
  pluginEnabled: boolean;
  pluginManaged: boolean;
  codexFound: boolean;
  codexVersion: string | null;
  codexVersionTested: boolean;
  codexPluginInstalled: boolean;
  codexPluginEnabled: boolean;
  codexPluginManaged: boolean;
  codexHooksPending: boolean;
  conversationsSeen: number;
  onboardingDone: boolean;
}

/** VS Code-family editor and the status of its exact-terminal extension. */
export interface EditorStatus {
  bundle: string;
  name: string;
  extensionInstalled: boolean;
  upToDate: boolean;
}

/** Latest release compared with the installed version. */
export interface UpdateState {
  current: string;
  latest: string | null;
  url: string | null;
  available: boolean;
  /** The companion shows the speech bubble until you click it. */
  announce: boolean;
  checking: boolean;
  installing: boolean;
  checkedAt: number | null;
  error: string | null;
  /** Why the app can't update itself. */
  blocker: string | null;
}

export interface Check {
  id: string;
  label: string;
  level: "ok" | "warn" | "error";
  detail: string;
  advanced: string | null;
}
