<script lang="ts">
  import { api } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import Robot from "../lib/Robot.svelte";
  import { failure, toast } from "../lib/ui.svelte";
  import type { Settings, SetupState } from "../lib/types";
  import type { Route } from "./routes";

  let { refresh, go }: { refresh: number; go: (r: Route) => void } = $props();

  const STEPS = ["Welcome", "Connection", "Summaries", "Notifications", "Test"];
  let step = $state(0);
  let setup = $state<SetupState | null>(null);
  let settings = $state<Settings | null>(null);
  let keyPresent = $state(false);
  let apiKey = $state("");
  let connecting = $state<"claude" | "codex" | null>(null);
  let connectError = $state<{ agent: "claude" | "codex"; message: string } | null>(null);
  let probe = $state<"idle" | "running" | "ok" | "failed">("idle");
  let initialSeen = -1;
  let seenNew = $state(false);

  api.settings().then((v) => {
    settings = v.settings;
    keyPresent = v.apiKeyPresent;
  }, failure);

  async function detect() {
    try {
      setup = await api.refreshEnvironment();
      if (initialSeen < 0) initialSeen = setup.conversationsSeen;
    } catch (e) {
      failure(e);
    }
  }
  detect();

  // During the test: the first real conversation observed.
  $effect(() => {
    void refresh;
    if (step === 4)
      api.setupState().then((s) => {
        if (initialSeen >= 0 && s.conversationsSeen > initialSeen) seenNew = true;
      });
  });

  async function connect(agent: "claude" | "codex") {
    connecting = agent;
    connectError = null;
    try {
      setup = agent === "claude" ? await api.connectClaude() : await api.connectCodex();
    } catch (e) {
      connectError = { agent, message: String(e) };
    } finally {
      connecting = null;
    }
  }

  async function saveSettings(patch: Partial<Settings>) {
    if (!settings) return;
    settings = { ...settings, ...patch };
    await api.saveSettings(settings).catch(failure);
  }

  async function saveKey() {
    if (!apiKey.trim()) return;
    await api.setApiKey(apiKey.trim()).catch(failure);
    keyPresent = true;
    apiKey = "";
    toast("Key saved in the keychain.");
  }

  async function runProbe() {
    probe = "running";
    try {
      const nonce = await api.startProbe();
      for (let i = 0; i < 60; i++) {
        await new Promise((r) => setTimeout(r, 1000));
        if (await api.probeStatus(nonce)) return void (probe = "ok");
      }
      probe = "failed";
    } catch (e) {
      probe = "failed";
      failure(e);
    }
  }

  async function finish() {
    await saveSettings({ onboardingDone: true });
    go({ name: "overview" });
  }

  const connected = $derived(!!setup && (setup.pluginEnabled || (setup.codexPluginEnabled && !setup.codexHooksPending)));
</script>

<div class="wizard">
  <ol class="steps" aria-label="Steps">
    {#each STEPS as label, i}
      <li class:done={i < step} class:current={i === step} aria-current={i === step ? "step" : undefined}>
        <span class="sr-only">{label}</span>
      </li>
    {/each}
  </ol>

  <div class="content">
    {#if step === 0}
      <Robot mood="rest" size={140} />
      <h1>Connect your agents</h1>
      <p>I collect your Claude Code and Codex conversations, group them by project and let you know when an agent is working, finishes or is waiting for you.</p>
      <p class="small">Everything stays on this Mac. It only takes a few minutes.</p>
    {:else if step === 1}
      <h1>Your agents</h1>
      {#if !setup?.detected}
        <p>Looking for Claude Code and Codex…</p>
      {:else if !setup.claudeFound && !setup.codexFound}
        <div class="status bad"><Icon name="error" /><span>Can't find Claude Code or Codex on this Mac. Install at least one, then try again.</span></div>
        <button class="btn secondary" onclick={detect}>Try again</button>
      {:else}
        <p>To connect an agent, I register a small component bundled with the app. It doesn't need GitHub access, doesn't change your settings, and you can disconnect it at any time.</p>
        <section class="agent">
          <h2>Claude Code</h2>
          {#if !setup.claudeFound}
            <p class="small">Not installed on this Mac.</p>
          {:else}
            <div class="status ok"><Icon name="check" /><span>Found Claude Code {setup.claudeVersion ?? ""}</span></div>
            {#if !setup.versionTested}
              <div class="status warn"><Icon name="warn" /><span>This version hasn't been verified yet: some statuses may be missing.</span></div>
            {/if}
            {#if setup.pluginEnabled}
              <div class="status ok"><Icon name="check" /><span>Connected to Flow Keeper</span></div>
            {:else}
              <button class="btn" disabled={!!connecting} onclick={() => connect("claude")}>{connecting === "claude" ? "Connecting…" : "Connect Claude Code"}</button>
            {/if}
          {/if}
        </section>
        <section class="agent">
          <h2>Codex</h2>
          {#if !setup.codexFound}
            <p class="small">Not installed on this Mac.</p>
          {:else}
            <div class="status ok"><Icon name="check" /><span>Found Codex {setup.codexVersion ?? ""}</span></div>
            {#if !setup.codexVersionTested}
              <div class="status warn"><Icon name="warn" /><span>This version hasn't been verified yet: some statuses may be missing.</span></div>
            {/if}
            {#if setup.codexPluginEnabled && setup.codexHooksPending}
              <div class="status warn">
                <Icon name="warn" />
                <span>One last step, just once: open Codex in a terminal, type <code>/hooks</code> and approve the Flow Keeper hooks. Until you approve them, Codex won't run them.</span>
              </div>
              <button class="btn secondary" onclick={detect}>I approved them, check again</button>
            {:else if setup.codexPluginEnabled}
              <div class="status ok"><Icon name="check" /><span>Connected to Flow Keeper</span></div>
            {:else}
              <button class="btn" disabled={!!connecting} onclick={() => connect("codex")}>{connecting === "codex" ? "Connecting…" : "Connect Codex"}</button>
            {/if}
          {/if}
        </section>
        {#if connectError}
          <div class="status bad"><Icon name="error" /><span>Couldn't connect {connectError.agent === "claude" ? "Claude Code" : "Codex"}. Try again; if the problem persists, open the details.</span></div>
          <details><summary>Details</summary><code class="block">{connectError.message}</code></details>
        {/if}
      {/if}
    {:else if step === 2 && settings}
      <h1>Summaries</h1>
      <p>For each conversation I prepare a topic and a short summary. Choose how to generate them.</p>
      <div class="choices">
        {#each [["claude_cli", "With Claude Code", "Uses your subscription. No key needed."], ["api", "With an API key", "Anthropic key in the keychain."], ["off", "Later", "Statuses and projects work without them too."]] as [value, label, hint]}
          <label class="choice" class:selected={settings.recapProvider === value}>
            <input type="radio" name="p" checked={settings.recapProvider === value} onchange={() => saveSettings({ recapProvider: value as Settings["recapProvider"] })} />
            <span><strong>{label}</strong><small>{hint}</small></span>
          </label>
        {/each}
      </div>
      {#if settings.recapProvider === "api"}
        <div class="inline">
          <input class="field" type="password" placeholder={keyPresent ? "Key already saved" : "sk-ant-…"} bind:value={apiKey} />
          <button class="btn small" onclick={saveKey} disabled={!apiKey.trim()}>Save</button>
        </div>
      {/if}
      <p class="small">With summaries on, portions of your conversations are sent to the model. You can exclude individual projects.</p>
    {:else if step === 3 && settings}
      <h1>Notifications</h1>
      <p>I only notify you when it matters. No sounds.</p>
      <label class="toggle"><span>When an agent is waiting for you</span><input type="checkbox" checked={settings.notifyAttention} onchange={(e) => saveSettings({ notifyAttention: e.currentTarget.checked })} /></label>
      <label class="toggle"><span>When an agent finishes a turn</span><input type="checkbox" checked={settings.notifyFinished} onchange={(e) => saveSettings({ notifyFinished: e.currentTarget.checked })} /></label>
      <button class="btn secondary" onclick={() => api.requestNotifications().then((ok) => toast(ok ? "Notifications allowed." : "You can allow them later in System Settings."))}>Allow notifications</button>
    {:else if step === 4}
      <h1>Let's try it</h1>
      <ul class="checklist">
        <!-- The automatic test uses a short Claude Code session. -->
        {#if setup?.pluginEnabled}
        <li class:ok={probe === "ok"}>
          <span class="mark">{#if probe === "ok"}<Icon name="check" size={16} />{/if}</span>
          <span>
            <strong>Connection verified</strong>
            <small>{probe === "running" ? "Starting a short test session…" : probe === "failed" ? "No event received. Try again or check the connections." : "An automatic session lasting a few seconds."}</small>
          </span>
          {#if probe !== "ok"}<button class="btn small" disabled={probe === "running"} onclick={runProbe}>Start</button>{/if}
        </li>
        {/if}
        <li class:ok={seenNew}>
          <span class="mark">{#if seenNew}<Icon name="check" size={16} />{/if}</span>
          <span>
            <strong>First conversation</strong>
            <small>{seenNew ? "There it is in Flow Keeper." : "Open a terminal, start Claude Code or Codex and type something: it will show up on its own."}</small>
          </span>
        </li>
      </ul>
      <p class="small">Then try removing it with “Remove from view” and typing in the terminal again: it will come back to its place.</p>
    {/if}
  </div>

  <footer>
    {#if step > 0}<button class="btn ghost" onclick={() => step--}>Back</button>{:else}<span></span>{/if}
    {#if step < STEPS.length - 1}
      <button class="btn" disabled={step === 1 && !connected} onclick={() => step++}>{step === 0 ? "Get started" : "Next"}</button>
    {:else}
      <button class="btn" onclick={finish}>Done</button>
    {/if}
  </footer>
  {#if step === 1 && !connected}
    <button class="skip" onclick={() => step++}>Skip for now</button>
  {/if}
</div>

<style>
  .wizard {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 18px 22px 16px;
    min-height: 0;
  }
  .steps {
    display: flex;
    gap: 6px;
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
  }
  .steps li {
    flex: 1;
    height: 4px;
    border-radius: 999px;
    background: var(--line);
  }
  .steps li.done,
  .steps li.current {
    background: var(--primary);
  }
  .steps li.current {
    opacity: 0.55;
  }
  .content {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 18px;
  }
  .content :global(.robot) {
    align-self: center;
    margin: 10px 0;
  }
  .agent {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .agent h2 {
    margin: 4px 0 0;
    font-size: 15px;
    font-weight: 650;
  }
  .agent .btn {
    align-self: flex-start;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  p {
    margin: 0;
    color: var(--text-2);
    font-size: 14px;
  }
  .small {
    font-size: 12.5px;
    color: var(--text-3);
  }
  .status {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 10px 12px;
    border-radius: var(--radius-md);
    font-size: 13.5px;
  }
  .status :global(svg) {
    flex-shrink: 0;
  }
  .status.ok {
    background: var(--finished-bg);
    color: var(--finished);
  }
  .status.warn {
    background: var(--waiting-bg);
    color: var(--waiting);
  }
  .status.bad {
    background: #fde7e5;
    color: var(--danger);
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .choice,
  .toggle {
    display: flex;
    align-items: center;
    gap: 12px;
    justify-content: space-between;
    background: var(--surface);
    border-radius: var(--radius-md);
    padding: 12px 14px;
    box-shadow: var(--shadow-card);
    border: 1.5px solid transparent;
  }
  .choice {
    justify-content: flex-start;
  }
  .choice.selected {
    border-color: var(--primary);
  }
  .choice span {
    display: flex;
    flex-direction: column;
  }
  small {
    color: var(--text-3);
    font-size: 12.5px;
  }
  .inline {
    display: flex;
    gap: 8px;
  }
  .checklist {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .checklist li {
    display: flex;
    gap: 12px;
    align-items: center;
    background: var(--surface);
    border-radius: var(--radius-md);
    padding: 12px 14px;
    box-shadow: var(--shadow-card);
  }
  .checklist li > span:nth-child(2) {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .mark {
    width: 24px;
    height: 24px;
    border-radius: 999px;
    border: 1.5px solid var(--line);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .checklist li.ok .mark {
    background: var(--finished);
    border-color: var(--finished);
    color: #fff;
  }
  code {
    font-size: 12px;
  }
  .block {
    display: block;
    margin-top: 6px;
    font-size: 11px;
    white-space: pre-wrap;
    -webkit-user-select: text;
    user-select: text;
  }
  footer {
    display: flex;
    justify-content: space-between;
    padding-top: 12px;
  }
  .skip {
    align-self: center;
    margin-top: 6px;
    border: 0;
    background: transparent;
    color: var(--text-3);
    font-size: 12.5px;
    text-decoration: underline;
  }
</style>
