<script lang="ts">
  import { api } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import { failure } from "../lib/ui.svelte";
  import type { Check } from "../lib/types";
  import type { Route } from "./routes";

  let { go }: { go: (r: Route) => void } = $props();
  let checks = $state<Check[] | null>(null);
  let advanced = $state(false);
  let probe = $state<"idle" | "running" | "ok" | "failed">("idle");

  async function run() {
    checks = null;
    try {
      checks = await api.doctor();
    } catch (e) {
      failure(e);
    }
  }
  run();

  async function startProbe() {
    probe = "running";
    try {
      const nonce = await api.startProbe();
      for (let i = 0; i < 60; i++) {
        await new Promise((r) => setTimeout(r, 1000));
        if (await api.probeStatus(nonce)) {
          probe = "ok";
          run();
          return;
        }
      }
      probe = "failed";
    } catch (e) {
      probe = "failed";
      failure(e);
    }
  }
</script>

<header class="top">
  <button class="back" onclick={() => go({ name: "settings" })}><Icon name="back" size={16} /> Settings</button>
</header>
<div class="title">
  <h1>Check connections</h1>
</div>

<div class="scroll">
  {#if !checks}
    <p class="muted">Checking…</p>
  {:else}
    <ul class="checks">
      {#each checks as c (c.id)}
        <li class={c.level}>
          <span class="icon"><Icon name={c.level === "ok" ? "check" : c.level === "warn" ? "warn" : "error"} /></span>
          <span class="text">
            <strong>{c.label}</strong>
            <span>{c.detail}</span>
            {#if advanced && c.advanced}<code>{c.advanced}</code>{/if}
          </span>
        </li>
      {/each}
    </ul>
  {/if}

  <section class="probe">
    <div>
      <strong>Guided test</strong>
      <p>Starts a short Claude Code session and checks that the event reaches Flow Keeper. It uses a small request to the cheapest model.</p>
      {#if probe === "ok"}<p class="ok">Event received: the connection works.</p>{/if}
      {#if probe === "failed"}<p class="bad">No event received within a minute. Check that the connection is active.</p>{/if}
    </div>
    <button class="btn small" disabled={probe === "running"} onclick={startProbe}>{probe === "running" ? "Running…" : "Start test"}</button>
  </section>

  <div class="foot">
    <button class="btn secondary small" onclick={run}>Check again</button>
    <label class="adv"><input type="checkbox" bind:checked={advanced} /> Show technical details</label>
  </div>
</div>

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
    gap: 14px;
  }
  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  li {
    display: flex;
    gap: 12px;
    background: var(--surface);
    border-radius: var(--radius-md);
    padding: 12px 14px;
    box-shadow: var(--shadow-card);
  }
  .icon {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    border-radius: 999px;
    display: grid;
    place-items: center;
  }
  .ok .icon {
    background: var(--finished-bg);
    color: var(--finished);
  }
  .warn .icon {
    background: var(--waiting-bg);
    color: var(--waiting);
  }
  .error .icon {
    background: #fde7e5;
    color: var(--danger);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 13px;
    color: var(--text-2);
    min-width: 0;
  }
  .text strong {
    color: var(--text);
    font-size: 14px;
  }
  code {
    margin-top: 4px;
    font-size: 11px;
    color: var(--text-3);
    word-break: break-all;
    -webkit-user-select: text;
    user-select: text;
  }
  .probe {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    justify-content: space-between;
    background: var(--surface);
    border-radius: var(--radius-md);
    padding: 14px;
    box-shadow: var(--shadow-card);
  }
  .probe p {
    margin: 4px 0 0;
    font-size: 12.5px;
    color: var(--text-2);
  }
  .probe .ok {
    color: var(--finished);
    font-weight: 600;
  }
  .probe .bad {
    color: var(--danger);
    font-weight: 600;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .adv {
    font-size: 12.5px;
    color: var(--text-2);
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .muted {
    color: var(--text-3);
  }
</style>
