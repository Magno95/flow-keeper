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
  /** First record of the transcript, or when the app first saw the conversation. */
  startedAt: number;
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
  /** Sends the use counters to `activityEndpoint`. */
  activitySync: boolean;
  activityEndpoint: string;
  activityToken: string;
  /** Changed by the user: the build's default server no longer applies. */
  activityCustom: boolean;
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
  claudeConversationsSeen: number;
  codexConversationsSeen: number;
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
  /** The releases after the installed one, newest first. */
  whatsNew: ReleaseNotes[];
}

/** A release's main changes, from its notes. */
export interface ReleaseNotes {
  version: string;
  highlights: string[];
}

/** Tokens as the agent reports them; `input` excludes cached tokens. */
export interface Tokens {
  input: number;
  cacheWrite: number;
  /** Part of `cacheWrite` kept for an hour, which costs more. */
  cacheWrite1h: number;
  cacheRead: number;
  output: number;
  /** Part of `output` spent on reasoning. */
  reasoning: number;
}

export interface Usage {
  /** Whole conversation, subagents included. */
  tokens: Tokens;
  /** Share spent by subagents (Claude Code only). */
  subagents: Tokens | null;
  requests: number;
  models: string[];
  /** The agent deleted the transcript: the figures are the ones Flow Keeper saved. */
  fromHistory: boolean;
}

export interface HourTokens {
  hourStart: number;
  tokens: Tokens;
  /** Estimated, in USD: Claude models only. */
  cost: number;
}

/** A project's or a model's part of the usage. */
export interface TokenShare {
  /** Project id; null for Unassigned and for models. */
  id: number | null;
  name: string;
  tokens: Tokens;
  cost: number;
  requests: number;
  conversations: number;
}

/** Tokens of all conversations, as saved by Flow Keeper. */
export interface TokenStats {
  /** First hour saved. */
  firstHour: number | null;
  tokens: Tokens;
  /** Estimated, in USD, at the Claude API list prices: Claude models only. */
  cost: number;
  requests: number;
  conversations: number;
  /** The period before, of the same length. */
  previous: Tokens | null;
  previousCost: number | null;
  hours: HourTokens[];
  /** Most used first. */
  projects: TokenShare[];
  models: TokenShare[];
  /** Claude models used in the period that have no price: not in `cost`. */
  unpriced: string[];
}

export interface Check {
  id: string;
  label: string;
  level: "ok" | "warn" | "error";
  detail: string;
  advanced: string | null;
}

/** Use of the app in one hour (start in UTC ms). */
export interface ActivityHour {
  hourStart: number;
  panelOpens: number;
  conversationOpens: number;
}

export interface Activity {
  hours: ActivityHour[];
  /** First hour counted on this Mac. */
  since: number | null;
}

/** Sending the counters to the server set in the settings. */
export interface ActivityStatus {
  pendingHours: number;
  lastSyncAt: number | null;
  lastAttemptAt: number | null;
  lastError: string | null;
}
