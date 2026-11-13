// Realistic mock data for reviewing the UI outside Tauri
// (`npm run dev` in the browser). Not used in the app.
import type { Activity, ActivityHour, ActivityStatus, CompanionState, ConversationCard, EditorStatus, Overview, ProjectCard, ProjectDetail, SetupState, Settings, Check, TokenShare, TokenStats, Tokens, UpdateState, Usage } from "./types";

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
    startedAt: now - 2 * 60 * min,
    lastActivityAt: now - 3 * min,
    hidden: false,
    attentionSeen: false,
    unread: false,
    canFocus: p.state !== "closed",
    canResume: p.state === "closed",
    ...p,
  };
}

const conversations: ConversationCard[] = [
  card({ id: 1, projectId: 1, title: "User sign-in", state: "working", summary: "Found the cause of the login error: the token expired before it was renewed. At the last update, tests were running.", environment: "iTerm", lastActivityAt: now - 1 * min }),
  card({ id: 2, projectId: 1, title: "Checkout with saved cards", state: "waiting", stateReason: "Asking for permission to run a command in the terminal", summary: "Added saving cards to the profile and updated the order summary.", folderLabel: "shop-checkout", lastActivityAt: now - 2 * min }),
  card({ id: 3, projectId: 1, title: "Migration to Vite 8", state: "finished", stateReason: "Response complete", summary: "Config updated and the build works; two CSS warnings still need fixing.", environment: "VS Code", agent: "Codex", unread: true, lastActivityAt: now - 12 * min }),
  card({ id: 4, projectId: 2, title: "Prices by market", state: "waiting", stateReason: "Has a question for you", summary: "Proposed two strategies for rounding prices in local currency.", lastActivityAt: now - 6 * min }),
  card({ id: 5, projectId: 2, title: "Rate cache", state: "closed", stateReason: "Session closed in the terminal", summary: "Added a 5-minute cache for rates, invalidated when the price list is updated.", lastActivityAt: now - 3 * 60 * min }),
  card({ id: 6, projectId: 3, title: "Review the Button components", titleIsProvisional: true, state: "idle", summaryStatus: "pending", summary: null, lastActivityAt: now - 2 * 60 * min }),
  card({ id: 7, projectId: null, title: "Script to rename photos", state: "finished", summary: "Created a script that renames photos by the date they were taken.", unread: true, lastActivityAt: now - 40 * min }),
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
    unread: list.filter((c) => c.unread).length,
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
  activitySync: false,
  activityEndpoint: "",
  activityToken: "",
  activityCustom: false,
};

const setup: SetupState = {
  detected: true,
  claudeFound: !location.search.includes("codex-only"),
  claudeVersion: "2.1.292",
  versionTested: true,
  pluginInstalled: true,
  pluginEnabled: !location.search.includes("disconnected") && !location.search.includes("codex-only"),
  pluginManaged: true,
  codexFound: true,
  codexVersion: "0.163.0",
  codexVersionTested: false,
  codexPluginInstalled: !location.search.includes("disconnected"),
  codexPluginEnabled: !location.search.includes("disconnected"),
  codexPluginManaged: true,
  codexHooksPending: location.search.includes("codex-review"),
  conversationsSeen: conversations.length,
  claudeConversationsSeen: 0,
  codexConversationsSeen: 0,
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
  whatsNew: [
    { version: "0.2.0", highlights: ["Each new version lists its main changes in Settings, next to Update now."] },
    {
      version: "0.1.1",
      highlights: [
        "Drag a project card to the left to remove the whole project from view.",
        "Usage from Claude Fable 5.2 is included in the estimated cost.",
      ],
    },
  ],
};

// Two months of plausible use: workdays, office hours, a lunch break.
const activityHours: ActivityHour[] = (() => {
  const hour = 3_600_000;
  const out: ActivityHour[] = [];
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  const start = Math.floor(now / hour) * hour - 60 * 24 * hour;
  for (let t = start; t <= now; t += hour) {
    const d = new Date(t);
    const h = d.getHours();
    const weekday = d.getDay() !== 0 && d.getDay() !== 6;
    const busy = weekday && h >= 9 && h <= 18 && h !== 13 ? 1 : h >= 21 && h <= 23 ? 0.15 : 0;
    if (rnd() > busy) continue;
    const panelOpens = 1 + Math.floor(rnd() * 6);
    out.push({ hourStart: t, panelOpens, conversationOpens: Math.floor(panelOpens * rnd() * 1.4) });
  }
  return out;
})();
const activityStatus: ActivityStatus = { pendingHours: 3, lastSyncAt: null, lastAttemptAt: null, lastError: null };

// Four months of tokens on the mock projects, Opus most of all.
const tokenRows = (() => {
  const hour = 3_600_000;
  const named: [number | null, string][] = [...projects.map((p): [number, string] => [p.id, p.name]), [null, "Unassigned"]];
  const models = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-4-5-20251001", "gpt-6-astra"];
  // Base input, cache read and output per million tokens; Codex has no price.
  const prices: (number[] | null)[] = [[4, 0.2, 20], [2, 0.1, 10], [1, 0.1, 5], null];
  const out: { hourStart: number; project: number; model: number; conversation: number; tokens: Tokens; cost: number }[] = [];
  let seed = 11;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  const start = Math.floor(now / hour) * hour - 120 * 24 * hour;
  for (let t = start; t <= now; t += hour) {
    const d = new Date(t);
    const weekday = d.getDay() !== 0 && d.getDay() !== 6;
    if (!weekday || d.getHours() < 9 || d.getHours() > 19 || rnd() > 0.55) continue;
    const scale = 0.3 + rnd() * 1.7;
    const output = Math.round(4000 + rnd() * 30_000 * scale);
    const cacheWrite = Math.round(output * 9 * rnd());
    const tokens = {
      input: Math.round(rnd() * 900),
      cacheWrite,
      cacheWrite1h: Math.round(cacheWrite * 0.8),
      cacheRead: output * 160,
      output,
      reasoning: Math.round(output * 0.4),
    };
    const model = Math.floor(rnd() ** 2.2 * models.length);
    const p = prices[model];
    const cost = p
      ? (tokens.input * p[0] + (cacheWrite - tokens.cacheWrite1h) * p[0] * 1.25 + tokens.cacheWrite1h * p[0] * 2 + tokens.cacheRead * p[1] + output * p[2]) / 1e6
      : 0;
    out.push({ hourStart: t, project: Math.floor(rnd() ** 1.6 * named.length), model, conversation: Math.floor(t / (4 * hour)), tokens, cost });
  }
  return { rows: out, projects: named, models };
})();

function tokenStats(since: number | null, previousSince: number | null, previousUntil: number | null): TokenStats {
  const zero = (): Tokens => ({ input: 0, cacheWrite: 0, cacheWrite1h: 0, cacheRead: 0, output: 0, reasoning: 0 });
  const add = (a: Tokens, b: Tokens) => {
    for (const k of Object.keys(a) as (keyof Tokens)[]) a[k] += b[k];
  };
  const { rows, projects, models } = tokenRows;
  const current = rows.filter((r) => since === null || r.hourStart >= since);
  const tokens = zero();
  let cost = 0;
  const hours = new Map<number, { tokens: Tokens; cost: number }>();
  const shares = (key: "project" | "model", name: (i: number) => [number | null, string]) => {
    const by = new Map<number, TokenShare & { ids: Set<number> }>();
    for (const r of current) {
      const [id, label] = name(r[key]);
      const s = by.get(r[key]) ?? { id, name: label, tokens: zero(), cost: 0, requests: 0, conversations: 0, ids: new Set<number>() };
      add(s.tokens, r.tokens);
      s.cost += r.cost;
      s.requests += 3;
      s.ids.add(r.conversation);
      by.set(r[key], s);
    }
    return [...by.values()]
      .map(({ ids, ...s }) => ({ ...s, conversations: ids.size }))
      .sort((a, b) => b.tokens.output + b.tokens.cacheRead - (a.tokens.output + a.tokens.cacheRead));
  };
  for (const r of current) {
    add(tokens, r.tokens);
    cost += r.cost;
    const h = hours.get(r.hourStart) ?? { tokens: zero(), cost: 0 };
    add(h.tokens, r.tokens);
    h.cost += r.cost;
    hours.set(r.hourStart, h);
  }
  let previous: Tokens | null = null;
  let previousCost: number | null = null;
  if (since !== null && previousSince !== null) {
    previous = zero();
    previousCost = 0;
    for (const r of rows) {
      if (r.hourStart < previousSince || r.hourStart >= (previousUntil ?? since)) continue;
      add(previous, r.tokens);
      previousCost += r.cost;
    }
  }
  return {
    firstHour: rows[0]?.hourStart ?? null,
    tokens,
    cost,
    requests: current.length * 3,
    conversations: new Set(current.map((r) => r.conversation)).size,
    previous,
    previousCost,
    hours: [...hours].map(([hourStart, h]) => ({ hourStart, ...h })),
    projects: shares("project", (i) => projects[i]),
    models: shares("model", (i) => [null, models[i]]),
    unpriced: [],
  };
}

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
    case "hide_project": {
      const list = visible().filter((c) => c.projectId === ((args.id as number | null) ?? null));
      for (const c of list) c.hidden = true;
      return list.map((c) => c.id);
    }
    case "unhide_conversation":
      conversations.find((c) => c.id === args.id)!.hidden = false;
      return;
    case "dismiss_attention": {
      const c = conversations.find((c) => c.id === args.id)!;
      if (c.state === "waiting") c.attentionSeen = true;
      return;
    }
    case "mark_project_read":
      for (const c of conversations) if (c.projectId === ((args.id as number | null) ?? null)) c.unread = false;
      return;
    case "list_projects":
      return projects;
    case "get_companion":
      return { mood: "attention", waiting: 2, working: 1 } satisfies CompanionState;
    case "get_settings":
      return { settings, apiKeyPresent: false };
    case "save_settings":
      Object.assign(settings, args.settings);
      return null;
    case "get_activity":
      return { hours: activityHours.filter((h) => h.hourStart >= (args.since as number)), since: activityHours[0].hourStart } satisfies Activity;
    case "get_activity_status":
      return { ...activityStatus };
    case "sync_activity":
      if (!settings.activityEndpoint.includes("ok")) {
        activityStatus.lastAttemptAt = Date.now();
        activityStatus.lastError = "server unreachable: dns failed";
        throw "server unreachable: dns failed";
      }
      Object.assign(activityStatus, { pendingHours: 0, lastSyncAt: Date.now(), lastAttemptAt: Date.now(), lastError: null });
      return { ...activityStatus };
    case "get_setup_state":
    case "refresh_environment":
    case "connect_claude":
    case "disconnect_claude":
    case "connect_codex":
    case "disconnect_codex":
      return { ...setup, onboardingDone: settings.onboardingDone };
    case "run_doctor":
      return checks;
    case "open_conversation":
      conversations.find((c) => c.id === args.id)!.unread = false;
      return { outcome: "unreachable", message: "The terminal window isn't responding.", command: "cd '/Users/me/shop' && claude --resume 06f55790-efe7-498b-b053-66a90e022f50" };
    case "resume_conversation":
      conversations.find((c) => c.id === args.id)!.unread = false;
      return { outcome: "resumed", app: "Terminal" };
    case "get_usage":
      return {
        tokens: { input: 1020, cacheWrite: 4_921_566, cacheWrite1h: 4_102_331, cacheRead: 77_988_005, output: 372_580, reasoning: 150_883 },
        subagents: { input: 404, cacheWrite: 3_381_306, cacheWrite1h: 2_950_120, cacheRead: 17_195_137, output: 59_778, reasoning: 41_109 },
        requests: 484,
        models: ["claude-opus-5-5"],
        fromHistory: false,
      } satisfies Usage;
    case "get_token_stats":
      return tokenStats(args.since as number | null, args.previousSince as number | null, (args.previousUntil as number | null) ?? null);
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
