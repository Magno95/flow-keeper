// Realistic mock data for reviewing the UI outside Tauri
// (`npm run dev` in the browser). Not used in the app.
import type { CompanionState, ConversationCard, EditorStatus, Overview, ProjectCard, ProjectDetail, SetupState, Settings, Check, UpdateState } from "./types";

const now = Date.now();
const min = 60_000;

function card(p: Partial<ConversationCard> & Pick<ConversationCard, "id" | "projectId" | "title" | "state">): ConversationCard {
  const labels: Record<string, string> = {
    working: "Working",
    waiting: "Waiting for you",
    finished: "Finished",
    idle: "No recent activity",
    closed: "Closed",
    unknown: "Status unavailable",
  };
  return {
    titleIsProvisional: false,
    summary: null,
    summaryStatus: "ready",
    summaryAt: now - 5 * min,
    stateLabel: labels[p.state],
    stateReason: null,
    agent: "Claude",
    environment: "Terminal",
    environmentIsGuess: false,
    folderLabel: null,
    lastActivityAt: now - 3 * min,
    hidden: false,
    attentionSeen: false,
    canFocus: p.state !== "closed",
    canResume: p.state === "closed",
    ...p,
  };
}

const conversations: ConversationCard[] = [
  card({ id: 1, projectId: 1, title: "User sign-in", state: "working", summary: "Found the cause of the login error: the token expired before it was renewed. At the last update, tests were running.", environment: "iTerm", lastActivityAt: now - 1 * min }),
  card({ id: 2, projectId: 1, title: "Checkout with saved cards", state: "waiting", stateReason: "Asking for permission to run a command in the terminal", summary: "Added saving cards to the profile and updated the order summary.", folderLabel: "shop-checkout", lastActivityAt: now - 2 * min }),
  card({ id: 3, projectId: 1, title: "Migration to Vite 8", state: "finished", stateReason: "Response complete", summary: "Config updated and the build works; two CSS warnings still need fixing.", environment: "VS Code", agent: "Codex", lastActivityAt: now - 12 * min }),
  card({ id: 4, projectId: 2, title: "Prices by market", state: "waiting", stateReason: "Has a question for you", summary: "Proposed two strategies for rounding prices in local currency.", lastActivityAt: now - 6 * min }),
  card({ id: 5, projectId: 2, title: "Rate cache", state: "closed", stateReason: "Session closed in the terminal", summary: "Added a 5-minute cache for rates, invalidated when the price list is updated.", lastActivityAt: now - 3 * 60 * min }),
  card({ id: 6, projectId: 3, title: "Review the Button components", titleIsProvisional: true, state: "idle", summaryStatus: "pending", summary: null, lastActivityAt: now - 2 * 60 * min }),
  card({ id: 7, projectId: null, title: "Script to rename photos", state: "finished", summary: "Created a script that renames photos by the date they were taken.", lastActivityAt: now - 40 * min }),
];

const projects = [
  { id: 1, name: "shop-frontend", description: "E-commerce site" },
  { id: 2, name: "pricing-api", description: null },
  { id: 3, name: "design-system", description: "Component library" },
];

function counts(list: ConversationCard[]) {
  return {
    total: list.length,
    open: list.filter((c) => c.state !== "closed").length,
    closed: list.filter((c) => c.state === "closed").length,
    working: list.filter((c) => c.state === "working").length,
    waiting: list.filter((c) => c.state === "waiting" && !c.attentionSeen).length,
    finished: list.filter((c) => c.state === "finished").length,
  };
}

function projectCard(id: number | null): ProjectCard {
  const list = conversations.filter((c) => c.projectId === id && !c.hidden);
  const p = projects.find((x) => x.id === id);
  return {
    id,
    name: p?.name ?? "Unassigned",
    description: p?.description ?? (id === null ? "Conversations outside a recognized repository" : null),
    recapExcluded: false,
    counts: counts(list),
    lastActivityAt: Math.max(...list.map((c) => c.lastActivityAt)),
  };
}

const settings: Settings = {
  recapProvider: "claude_cli",
  recapCliModel: "haiku",
  recapApiModel: "claude-haiku-4-5",
  recapDailyLimit: 300,
  recapMonthlyBudgetUsd: 10,
  notifyAttention: true,
  notifyFinished: true,
  doNotDisturb: false,
  trackPrintSessions: false,
  shortcut: "Alt+Super+J",
  companionVisible: true,
  launchAtLogin: true,
  onboardingDone: !location.search.includes("onboarding"),
};

const setup: SetupState = {
  detected: true,
  claudeFound: true,
  claudeVersion: "2.1.292",
  versionTested: true,
  pluginInstalled: true,
  pluginEnabled: !location.search.includes("disconnected"),
  pluginManaged: true,
  codexFound: true,
  codexVersion: "0.163.0",
  codexVersionTested: false,
  codexPluginInstalled: !location.search.includes("disconnected"),
  codexPluginEnabled: !location.search.includes("disconnected"),
  codexPluginManaged: true,
  codexHooksPending: location.search.includes("codex-review"),
  conversationsSeen: conversations.length,
  onboardingDone: settings.onboardingDone,
};

const checks: Check[] = [
  { id: "claude", label: "Claude Code", level: "ok", detail: "Version 2.1.292, verified.", advanced: "executable: /opt/homebrew/bin/claude" },
  { id: "plugin", label: "Connection with Claude Code", level: "ok", detail: "Active.", advanced: "plugin flow-keeper@flow-keeper 0.1.0" },
  { id: "codex", label: "Codex CLI", level: "warn", detail: "Version 0.163.0: not yet verified with Flow Keeper. Statuses may be incomplete.", advanced: "executable: ~/.local/bin/codex" },
  { id: "codex_plugin", label: "Connection with Codex", level: "ok", detail: "Active.", advanced: "plugin flow-keeper@flow-keeper-app 0.1.0; hooks approved 8/8" },
  { id: "spool", label: "Local collection", level: "ok", detail: "Working.", advanced: null },
  { id: "events", label: "Events received", level: "ok", detail: "Last event 1 minute ago.", advanced: null },
  { id: "recap", label: "Summaries", level: "warn", detail: "1 summary to update.", advanced: null },
];

const update: UpdateState = {
  current: "0.1.0",
  latest: "0.2.0",
  url: "https://github.com/Magno95/flow-keeper/releases/tag/v0.2.0",
  available: true,
  announce: true,
  checking: false,
  installing: false,
  checkedAt: Date.now() - 4 * 60000,
  error: null,
  blocker: null,
};

export async function mockInvoke(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
  await new Promise((r) => setTimeout(r, 60));
  const visible = () => conversations.filter((c) => !c.hidden);
  switch (cmd) {
    case "get_overview": {
      const ids = [...new Set(visible().map((c) => c.projectId))];
      const list = ids.map(projectCard).sort((a, b) => b.counts.waiting - a.counts.waiting || b.lastActivityAt - a.lastActivityAt);
      return {
        projects: location.search.includes("empty") ? [] : list,
        waiting: location.search.includes("empty") ? [] : visible().filter((c) => c.state === "waiting" && !c.attentionSeen),
        hiddenCount: conversations.filter((c) => c.hidden).length,
        totalConversations: location.search.includes("empty") ? 0 : visible().length,
      } satisfies Overview;
    }
    case "get_project": {
      const id = (args.id as number | null) ?? null;
      const order = { waiting: 0, working: 1, finished: 2, unknown: 3, idle: 4, closed: 5 };
      return {
        project: projectCard(id),
        conversations: visible().filter((c) => c.projectId === id).sort((a, b) => order[a.state] - order[b.state]),
      } satisfies ProjectDetail;
    }
    case "get_removed":
      return conversations.filter((c) => c.hidden);
    case "search": {
      const q = String(args.query).toLowerCase();
      return visible().filter((c) => (c.title + " " + (c.summary ?? "")).toLowerCase().includes(q));
    }
    case "hide_conversation":
      conversations.find((c) => c.id === args.id)!.hidden = true;
      return;
    case "unhide_conversation":
      conversations.find((c) => c.id === args.id)!.hidden = false;
      return;
    case "dismiss_attention": {
      const c = conversations.find((c) => c.id === args.id)!;
      if (c.state === "waiting") c.attentionSeen = true;
      return;
    }
    case "list_projects":
      return projects;
    case "get_companion":
      return { mood: "attention", waiting: 2, working: 1 } satisfies CompanionState;
    case "get_settings":
      return { settings, apiKeyPresent: false };
    case "get_setup_state":
    case "refresh_environment":
    case "connect_claude":
    case "disconnect_claude":
    case "connect_codex":
    case "disconnect_codex":
      return setup;
    case "run_doctor":
      return checks;
    case "open_conversation":
      return { outcome: "unreachable", message: "The terminal window isn't responding.", command: "cd '/Users/me/shop' && claude --resume 06f55790-efe7-498b-b053-66a90e022f50" };
    case "resume_conversation":
      return { outcome: "resumed", app: "Terminal" };
    case "start_probe":
      return "p1";
    case "probe_status":
      return true;
    case "get_editors":
    case "install_editor_extension":
    case "uninstall_editor_extension":
      return [{ bundle: "com.microsoft.VSCode", name: "VS Code", extensionInstalled: cmd === "install_editor_extension", upToDate: true }] satisfies EditorStatus[];
    case "get_update":
      return { ...update };
    case "check_update":
      update.checkedAt = Date.now();
      return { ...update };
    case "dismiss_update":
      update.announce = false;
      return null;
    case "install_update":
      update.installing = true;
      return null;
    default:
      return null;
  }
}
