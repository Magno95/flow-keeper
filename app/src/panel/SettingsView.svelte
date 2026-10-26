<script lang="ts">
  import { onMount } from "svelte";
  import { api, onUpdate } from "../lib/api";
  import { ago } from "../lib/format";
  import Icon from "../lib/Icon.svelte";
  import { failure, toast } from "../lib/ui.svelte";
  import type { EditorStatus, Settings, UpdateState } from "../lib/types";
  import type { Route } from "./routes";

  let { go }: { go: (r: Route) => void } = $props();
  let s = $state<Settings | null>(null);
  let keyPresent = $state(false);
  let apiKey = $state("");
  let connected = $state<boolean | null>(null);
  let codexConnected = $state<boolean | null>(null);
  api.setupState().then((v) => {
    connected = v.pluginEnabled;
    codexConnected = v.codexPluginInstalled && v.codexPluginManaged;
  }, () => {});

  async function disconnect() {
    try {
      const v = await api.disconnectClaude();
      connected = v.pluginEnabled;
      toast("Claude Code disconnected: new sessions will no longer be collected.");
    } catch (e) {
      failure(e);
    }
  }

  async function disconnectCodex() {
    try {
      const v = await api.disconnectCodex();
      codexConnected = v.codexPluginInstalled && v.codexPluginManaged;
      toast("Codex disconnected: new sessions will no longer be collected.");
    } catch (e) {
      failure(e);
    }
  }

  let editors = $state<EditorStatus[]>([]);
  let editorBusy = $state<string | null>(null);
  api.editors().then((v) => (editors = v), () => {});

  async function toggleExtension(e: EditorStatus) {
    editorBusy = e.bundle;
    try {
      editors = e.extensionInstalled ? await api.uninstallEditorExtension(e.bundle) : await api.installEditorExtension(e.bundle);
      toast(
        e.extensionInstalled
          ? `Extension removed from ${e.name}.`
          : `Extension installed in ${e.name}: windows that are already open pick it up after “Reload Window”.`,
      );
    } catch (err) {
      failure(err);
    } finally {
      editorBusy = null;
    }
  }

  let u = $state<UpdateState | null>(null);
  onMount(() => {
    api.update().then((v) => (u = v), () => {});
    const sub = onUpdate((v) => (u = v));
    return () => sub.then((f) => f());
  });

  const updateStatus = $derived.by(() => {
    if (!u) return "";
    const parts = [`Version ${u.current}`];
    if (u.checkedAt) parts.push(`last checked ${ago(u.checkedAt)}`);
    if (u.checkedAt && !u.available && !u.error) parts.push("up to date");
    return parts.join(" · ");
  });

  async function checkUpdate() {
    try {
      u = await api.checkUpdate();
      if (u.error) return;
      toast(u.available ? `Version ${u.latest} is available.` : "You already have the latest version.");
    } catch (e) {
      failure(e);
    }
  }

  async function installUpdate() {
    if (u) u = { ...u, installing: true };
    try {
      await api.installUpdate();
    } catch (e) {
      failure(e);
    }
  }

  api.settings().then((v) => {
    s = v.settings;
    keyPresent = v.apiKeyPresent;
  }, failure);

  async function save(patch: Partial<Settings>) {
    if (!s) return;
    s = { ...s, ...patch };
    try {
      await api.saveSettings(s);
    } catch (e) {
      failure(e);
    }
  }

  async function saveKey() {
    try {
      await api.setApiKey(apiKey.trim() || null);
      keyPresent = !!apiKey.trim();
      apiKey = "";
      toast(keyPresent ? "Key saved in the keychain." : "Key removed.");
    } catch (e) {
      failure(e);
    }
  }
</script>

<header class="top">
  <button class="back" onclick={() => go({ name: "overview" })}><Icon name="back" size={16} /> Projects</button>
</header>
<div class="title"><h1>Settings</h1></div>

{#snippet updates()}
  <section>
    <h2>Updates</h2>
    {#if u?.available}
      <div class="update">
        <span>
          <strong>{u.installing ? `Updating to version ${u.latest}…` : `Version ${u.latest} is available`}</strong>
          <small>
            {u.installing
              ? "Downloading and verifying the new bundle, then the app quits and reopens on its own."
              : `You have ${u.current}. The app quits, updates and reopens on its own; your data stays.`}
          </small>
        </span>
        <button class="btn small" disabled={u.installing || !!u.blocker} onclick={installUpdate}>
          {u.installing ? "Updating…" : "Update now"}
        </button>
      </div>
      {#if u.blocker}<p class="note">{u.blocker}.</p>{/if}
    {/if}
    {#if u}
      <button class="row-btn" disabled={u.checking || u.installing} onclick={checkUpdate}>
        <span><strong>{u.checking ? "Checking…" : "Check for updates"}</strong><small>{updateStatus}</small></span>
      </button>
      {#if u.error}<p class="note error">{u.error}</p>{/if}
    {/if}
  </section>
{/snippet}

{#if s}
  <div class="scroll">
    <!-- With a new version the section goes to the top; otherwise it stays at the bottom. -->
    {#if u?.available}{@render updates()}{/if}

    <section>
      <h2>Connections</h2>
      <button class="row-btn" onclick={() => go({ name: "doctor" })}>
        <span><strong>Check connections</strong><small>Verify agents, collection and summaries</small></span>
        <Icon name="chevron" />
      </button>
      <button class="row-btn" onclick={() => go({ name: "onboarding" })}>
        <span><strong>Connect your agents</strong><small>Run the guided setup again</small></span>
        <Icon name="chevron" />
      </button>
      {#if connected}
        <button class="row-btn" onclick={disconnect}>
          <span><strong>Disconnect Claude Code</strong><small>Removes the app's component; collected data stays</small></span>
        </button>
      {/if}
      {#if codexConnected}
        <button class="row-btn" onclick={disconnectCodex}>
          <span><strong>Disconnect Codex</strong><small>Removes the app's component; collected data stays</small></span>
        </button>
      {/if}
    </section>

    {#if editors.length}
      <section>
        <h2>Editors</h2>
        {#each editors as e (e.bundle)}
          <button class="row-btn" disabled={editorBusy === e.bundle} onclick={() => toggleExtension(e)}>
            <span>
              <strong>{e.extensionInstalled ? `Remove the extension from ${e.name}` : `Install the extension in ${e.name}`}</strong>
              <small>{e.extensionInstalled ? "“Open conversation” selects the exact terminal" : "So “Open conversation” selects the exact terminal, not just the window"}</small>
            </span>
          </button>
        {/each}
      </section>
    {/if}

    <section>
      <h2>Summaries</h2>
      <div class="choices" role="radiogroup" aria-label="Summary source">
        {#each [["claude_cli", "With Claude Code", "Uses your subscription, no key to set up."], ["api", "With an API key", "Anthropic key saved in the macOS keychain."], ["off", "Off", "Collection, projects and statuses still work."]] as [value, label, hint]}
          <label class="choice" class:selected={s.recapProvider === value}>
            <input type="radio" name="provider" checked={s.recapProvider === value} onchange={() => save({ recapProvider: value as Settings["recapProvider"] })} />
            <span><strong>{label}</strong><small>{hint}</small></span>
          </label>
        {/each}
      </div>
      {#if s.recapProvider === "api"}
        <div class="inline">
          <input class="field" type="password" placeholder={keyPresent ? "Key saved — enter a new one to replace it" : "sk-ant-…"} bind:value={apiKey} />
          <button class="btn small" onclick={saveKey}>{apiKey.trim() || !keyPresent ? "Save" : "Remove"}</button>
        </div>
      {/if}
      <div class="grid">
        <label>Calls per day<input class="field" type="number" min="0" value={s.recapDailyLimit} onchange={(e) => save({ recapDailyLimit: Number(e.currentTarget.value) })} /></label>
        <label>Monthly budget (USD)<input class="field" type="number" min="0" step="0.5" value={s.recapMonthlyBudgetUsd} onchange={(e) => save({ recapMonthlyBudgetUsd: Number(e.currentTarget.value) })} /></label>
      </div>
      <p class="note">Conversations are summarized by sending portions of text to the chosen model. You can exclude a project from its menu. Check your company policies before enabling summaries on work projects.</p>
    </section>

    <section>
      <h2>Notifications</h2>
      <label class="toggle"><span>When an agent is waiting for you</span><input type="checkbox" checked={s.notifyAttention} onchange={(e) => save({ notifyAttention: e.currentTarget.checked })} /></label>
      <label class="toggle"><span>When an agent finishes a turn</span><input type="checkbox" checked={s.notifyFinished} onchange={(e) => save({ notifyFinished: e.currentTarget.checked })} /></label>
      <label class="toggle"><span>Do not disturb</span><input type="checkbox" checked={s.doNotDisturb} onchange={(e) => save({ doNotDisturb: e.currentTarget.checked })} /></label>
    </section>

    <section>
      <h2>FloK</h2>
      <label class="toggle"><span>Show FloK on the desktop</span><input type="checkbox" checked={s.companionVisible} onchange={(e) => save({ companionVisible: e.currentTarget.checked })} /></label>
      <label class="toggle"><span>Launch at login</span><input type="checkbox" checked={s.launchAtLogin} onchange={(e) => save({ launchAtLogin: e.currentTarget.checked })} /></label>
      <label class="stack">Shortcut to open Flow Keeper<input class="field" value={s.shortcut} onchange={(e) => save({ shortcut: e.currentTarget.value })} /></label>
      <p class="note">Format like “Alt+Super+J” (⌥⌘J). Hiding FloK doesn't stop collection: the menu bar icon stays.</p>
    </section>

    {#if !u?.available}{@render updates()}{/if}

    <section>
      <button class="btn secondary" onclick={() => api.quit()}>Quit Flow Keeper</button>
    </section>
  </div>
{/if}

<style>
  .top {
    padding: 14px 14px 0 12px;
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-weight: 600;
    font-size: 13.5px;
    padding: 6px 8px;
    border-radius: 999px;
  }
  .title {
    padding: 6px 20px 8px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h2 {
    margin: 0 0 2px;
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-3);
  }
  .row-btn,
  .choice,
  .toggle,
  .update {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    text-align: left;
    border: 0;
    background: var(--surface);
    border-radius: var(--radius-md);
    padding: 12px 14px;
    box-shadow: var(--shadow-card);
    font-size: 14px;
  }
  .row-btn span,
  .choice span,
  .update span {
    display: flex;
    flex-direction: column;
  }
  small {
    color: var(--text-3);
    font-size: 12.5px;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .choice {
    justify-content: flex-start;
    border: 1.5px solid transparent;
  }
  .choice.selected {
    border-color: var(--primary);
  }
  .inline {
    display: flex;
    gap: 8px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .grid label,
  .stack {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12.5px;
    color: var(--text-2);
  }
  .note {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-3);
  }
  .note.error {
    color: var(--danger);
  }
  .update {
    border: 1.5px solid var(--working);
  }
  .update .btn {
    flex: none;
  }
</style>
