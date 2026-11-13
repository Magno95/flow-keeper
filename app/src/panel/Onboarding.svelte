<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import Robot from "../lib/Robot.svelte";
  import { failure, toast } from "../lib/ui.svelte";
  import type { EditorStatus, Settings, SetupState } from "../lib/types";
  import type { Route } from "./routes";

  let { refresh, go }: { refresh: number; go: (r: Route) => void } = $props();

  const STEPS = ["Welcome", "Agents", "Editors", "Summaries", "Notifications", "Test", "Review"];
  let step = $state(0);
  let setup = $state<SetupState | null>(null);
  let settings = $state<Settings | null>(null);
  let keyPresent = $state(false);
  let apiKey = $state("");
  let connecting = $state<"claude" | "codex" | null>(null);
  let connectError = $state<{ agent: "claude" | "codex"; message: string } | null>(null);
  let probe = $state<"idle" | "running" | "ok" | "failed">("idle");
  let baseline: SetupState | null = null;
  let editors = $state<EditorStatus[]>([]);
  let editorBusy = $state<string | null>(null);
  let editorsLoaded = $state(false);
  let error = $state("");
  let saving = $state(false);
  let notifications = $state<"unchecked" | "allowed" | "denied">("unchecked");
  let heading: HTMLHeadingElement;
  const agents = ["claude", "codex"] as const;
  const names = { claude: "Claude Code", codex: "Codex" };
  let verified = $state({ claude: false, codex: false });

  async function loadEditors() {
    try { editors = await api.editors(); editorsLoaded = true; }
    catch (e) { error = String(e); }
  }
  loadEditors();

  async function installEditor(bundle: string) {
    editorBusy = bundle;
    error = "";
    try { editors = await api.installEditorExtension(bundle); }
    catch (e) { error = String(e); }
    finally { editorBusy = null; }
  }

  function ready(agent: "claude" | "codex") {
    return agent === "claude" ? !!setup?.pluginEnabled : !!setup?.codexPluginEnabled && !setup.codexHooksPending;
  }

  async function poll() {
    try {
      const next = await api.setupState();
      if (baseline) {
        verified.claude ||= next.claudeConversationsSeen > baseline.claudeConversationsSeen;
        verified.codex ||= next.codexConversationsSeen > baseline.codexConversationsSeen;
      }
      setup = next;
    } catch (e) { error = String(e); }
  }
  onMount(() => {
    const timer = setInterval(() => { if (step >= 5) poll(); }, 2000);
    return () => clearInterval(timer);
  });

  async function move(next: number) {
    error = "";
    if (next > step && step === 3 && !summaryReady) {
      error = "Choose an available summary provider, save your API key, or select Later.";
      return;
    }
    step = next;
    await tick();
    heading?.focus();
  }

  async function allowNotifications() {
    try { notifications = await api.requestNotifications() ? "allowed" : "denied"; }
    catch (e) { error = String(e); }
  }

  async function loadSettings() {
    try {
      const v = await api.settings();
      settings = v.settings;
      keyPresent = v.apiKeyPresent;
    } catch (e) { error = String(e); }
  }
  loadSettings();

  async function detect() {
    error = "";
    try {
      setup = await api.refreshEnvironment();
      if (!baseline) baseline = setup;
    } catch (e) {
      error = String(e);
    }
  }
  detect();

  $effect(() => {
    void refresh;
    if (step >= 5) poll();
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

  async function saveSettings(patch: Partial<Settings>): Promise<boolean> {
    if (!settings || saving) return false;
    saving = true;
    error = "";
    const next = { ...settings, ...patch };
    try {
      await api.saveSettings(next);
      settings = next;
      return true;
    } catch (e) { error = String(e); return false; }
    finally { saving = false; }
  }

  async function saveKey() {
    if (!apiKey.trim() || saving) return;
    saving = true;
    error = "";
    try {
      await api.setApiKey(apiKey.trim());
      keyPresent = true;
      apiKey = "";
      toast("Key saved in the keychain.");
    } catch (e) { error = String(e); }
    finally { saving = false; }
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
    if (await saveSettings({ onboardingDone: true })) go({ name: "overview" });
  }

  const summaryReady = $derived(!!settings && (settings.recapProvider === "off" || (settings.recapProvider === "api" ? keyPresent && !apiKey.trim() : !!setup?.claudeFound)));
  const connected = $derived(ready("claude") || ready("codex"));
</script>

<div class="wizard">
  <ol class="steps" aria-label="Steps">
    {#each STEPS as label, i}
      <li class:done={i < step} class:current={i === step} aria-current={i === step ? "step" : undefined}>
        <span class="sr-only">{label}</span>
      </li>
    {/each}
  </ol>

  <p class="small" aria-live="polite">Step {step + 1} of {STEPS.length}: {STEPS[step]}</p>
  <h1 bind:this={heading} tabindex="-1" class="sr-only">{STEPS[step]}</h1>
  {#if error}<div class="status bad" role="alert">{error}</div>{/if}
  <div class="content">
    {#if step === 0}
      <Robot mood="rest" size={140} />
      <h1>Connect your agents</h1>
      <p>I collect your Claude Code and Codex conversations, group them by project and let you know when an agent is working, finishes or is waiting for you.</p>
      <p class="small">Conversations are collected on this Mac. Summaries can send text to a model; you choose that later.</p>
    {:else if step === 1}
      <h1>Your agents</h1>
      {#if !setup?.detected}
        <p>Looking for Claude Code and Codex…</p>
      {:else if !setup.claudeFound && !setup.codexFound}
        <div class="status bad"><Icon name="error" /><span>Can't find Claude Code or Codex on this Mac. Install and sign in to at least one CLI, then try again. Editor extensions and desktop apps alone are not sufficient.</span></div>
        <button class="btn secondary" onclick={detect}>Try again</button>
      {:else}
        <p>To connect an agent, I register a small component bundled with the app. It doesn't need GitHub access, doesn't change your settings, and you can disconnect it at any time.</p>
        <p class="small">Connect only the agents you use. After connecting, start a new session so it can load the plugin.</p>
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
                <span>One last step, just once: open Codex in a terminal, start a new session with <code>codex</code>, type <code>/hooks</code> and approve and enable the Flow Keeper hooks. Return here and check again. Until you approve them, Codex won't run them.</span>
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
      <button class="btn secondary" disabled={!!connecting} onclick={detect}>Check agents again</button>
    {:else if step === 2}
      <h1>Your editor (optional)</h1>
      <p>The Flow Keeper extension lets “Open conversation” select the exact integrated terminal. Agent collection works without it.</p>
      {#if !editorsLoaded}<p>Editors have not been detected yet.</p>{/if}
      {#if editorsLoaded && !editors.length}<p>No supported editor found. You can continue using Terminal.</p>{/if}
      {#each editors as editor (editor.bundle)}
        <section class="agent">
          <h2>{editor.name}</h2>
          {#if editor.extensionInstalled && editor.upToDate}
            <p>Extension installed. Activation in your window still needs to be checked.</p>
            <p class="small">In {editor.name}, press ⇧⌘P, run “Developer: Reload Window”, then start a new agent session in the integrated terminal. After setup, use “Open conversation” to check that the correct terminal is selected.</p>
          {:else}
            <button class="btn" disabled={!!editorBusy} onclick={() => installEditor(editor.bundle)}>{editorBusy === editor.bundle ? "Installing…" : editor.extensionInstalled ? "Update extension" : "Install extension"}</button>
          {/if}
        </section>
      {/each}
      <button class="btn secondary" disabled={!!editorBusy} onclick={loadEditors}>Check editors again</button>
      <p class="small">You can install this later in Settings → Editors.</p>
    {:else if step === 3 && settings}
      <h1>Summaries</h1>
      <p>For each conversation I prepare a topic and a short summary. Choose how to generate them.</p>
      <div class="choices">
        {#each [["claude_cli", "With Claude Code", "Uses your subscription. No key needed."], ["api", "With an API key", "Anthropic key in the keychain."], ["off", "Later", "Statuses and projects work without them too."]] as [value, label, hint]}
          <label class="choice" class:selected={settings.recapProvider === value}>
            <input type="radio" name="p" checked={settings.recapProvider === value} disabled={saving || (value === "claude_cli" && !setup?.claudeFound)} onchange={() => saveSettings({ recapProvider: value as Settings["recapProvider"] })} />
            <span><strong>{label}</strong><small>{hint}</small></span>
          </label>
        {/each}
      </div>
      {#if settings.recapProvider === "api"}
        <label for="onboarding-key">Anthropic API key</label>
        <div class="inline">
          <input class="field" type="password" id="onboarding-key" placeholder={keyPresent ? "Key already saved" : "sk-ant-…"} bind:value={apiKey} />
          <button class="btn small" onclick={saveKey} disabled={saving || !apiKey.trim()}>Save</button>
        </div>
      {/if}
      {#if !summaryReady}<p class="status warn">{settings.recapProvider === "api" ? "Save an API key, or choose Later to continue without summaries." : "Claude Code is unavailable. Choose an API key or Later."}</p>{/if}
      <p class="small">With summaries on, portions of your conversations are sent to the model. You can exclude individual projects.</p>
    {:else if step === 4 && settings}
      <h1>Notifications</h1>
      <p>I only notify you when it matters. No sounds.</p>
      <label class="toggle"><span>When an agent is waiting for you</span><input type="checkbox" disabled={saving} checked={settings.notifyAttention} onchange={(e) => saveSettings({ notifyAttention: e.currentTarget.checked })} /></label>
      <label class="toggle"><span>When an agent finishes a turn</span><input type="checkbox" disabled={saving} checked={settings.notifyFinished} onchange={(e) => saveSettings({ notifyFinished: e.currentTarget.checked })} /></label>
      <button class="btn secondary" onclick={allowNotifications}>Allow notifications</button>
      <p aria-live="polite">{notifications === "allowed" ? "Notifications allowed." : notifications === "denied" ? "Allow Flow Keeper in System Settings → Notifications, or continue without notifications." : "Permission has not been checked yet."}</p>
    {:else if step === 5}
      <h1>Let's try it</h1>
      <ul class="checklist">
        <!-- The automatic test uses a short Claude Code session. -->
        {#if setup?.pluginEnabled}
        <li class:ok={probe === "ok"}>
          <span class="mark">{#if probe === "ok"}<Icon name="check" size={16} />{/if}</span>
          <span>
            <strong>{probe === "ok" ? "Claude event delivery verified" : "Optional Claude connection test"}</strong>
            <small>{probe === "running" ? "Starting a short test session…" : probe === "failed" ? "No event received. Try again or check the connections." : "A short Claude request using Haiku; uses your account."}</small>
          </span>
          {#if probe !== "ok"}<button class="btn small" disabled={probe === "running"} onclick={runProbe}>Start</button>{/if}
        </li>
        {/if}
        {#each agents as agent}
          {#if ready(agent)}
            <li class:ok={verified[agent]}>
              <span class="mark">{#if verified[agent]}<Icon name="check" size={16} />{/if}</span>
              <span><strong>{names[agent]}</strong><small>{verified[agent] ? "A new conversation was received during this setup." : `In your terminal, start a new session with ${agent}, complete sign-in if requested, and send “Reply only: ok”. Return here; this check updates automatically.`}</small></span>
            </li>
          {/if}
        {/each}
      </ul>
      {#if !connected}<p>No agent is ready. Go back to Agents to connect one, or continue and finish later.</p>{/if}
      <p class="small">Test each agent you connected. Existing sessions may need to be restarted after installing the plugin.</p>
      <button class="btn secondary" onclick={poll}>Check again</button>
    {:else if step === 6}
      <h1>Review your setup</h1>
      {#each agents as agent}
        <section class="agent">
          <h2>{names[agent]}</h2>
          <p>{ready(agent) ? verified[agent] ? "Verified: new conversation received." : "Connected; real conversation test still pending." : agent === "codex" && setup?.codexPluginEnabled ? "Action required: approve and enable Flow Keeper in Codex /hooks, then check again in Agents." : "Not connected. You can connect it later."}</p>
        </section>
      {/each}
      {#each editors as editor}
        <p><strong>{editor.name}:</strong> {editor.extensionInstalled ? "Extension installed. Reload Window and verify that Open conversation selects your terminal." : "Extension skipped; available in Settings → Editors."}</p>
      {/each}
      {#if !editorsLoaded}<p>Editor detection was not completed. Retry in Editors or check Settings later.</p>{/if}
      <p><strong>Summaries:</strong> {settings?.recapProvider === "off" ? "Off." : summaryReady ? "Configured; generation has not been tested." : "Setup incomplete."}</p>
      <p><strong>Notifications:</strong> {notifications === "allowed" ? "Allowed." : notifications === "denied" ? "Enable in System Settings when you are ready." : "Permission not checked."}</p>
      <p class="small">You can finish with pending steps. Reopen Settings → Connect your agents to complete them. Editor activation and terminal selection must be checked in your own window.</p>
    {/if}
    {#if !settings && (step === 3 || step === 4)}<p>Settings could not be loaded yet.</p><button class="btn secondary" onclick={loadSettings}>Retry loading settings</button>{/if}
  </div>

  <footer>
    {#if step > 0}<button class="btn ghost" disabled={saving || !!connecting || !!editorBusy} onclick={() => move(step - 1)}>Back</button>{:else}<span></span>{/if}
    {#if step < STEPS.length - 1}
      <button class="btn" disabled={saving || !!connecting || !!editorBusy || ((step === 3 || step === 4) && !settings)} onclick={() => move(step + 1)}>{step === 0 ? "Get started" : "Next"}</button>
    {:else}
      <button class="btn" disabled={saving || !settings} onclick={finish}>{saving ? "Saving…" : "Finish setup"}</button>
    {/if}
  </footer>

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

</style>
