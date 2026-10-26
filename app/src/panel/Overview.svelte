<script lang="ts">
  import { api } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import Robot from "../lib/Robot.svelte";
  import { plural, stableOrder } from "../lib/format";
  import { failure, primaryAction } from "../lib/ui.svelte";
  import type { ConversationCard, Overview, SetupState } from "../lib/types";
  import ConversationCardView from "./ConversationCard.svelte";
  import ProjectCardView from "./ProjectCard.svelte";
  import type { Route } from "./routes";

  let { refresh, opened, setup, go }: { refresh: number; opened: number; setup: SetupState | null; go: (r: Route) => void } =
    $props();

  let data = $state<Overview | null>(null);
  let order: (number | string)[] = [];
  let query = $state("");
  let results = $state<ConversationCard[] | null>(null);
  let lastOpened = -1;

  const key = (id: number | null) => id ?? "unassigned";
  const projects = $derived(data ? stableOrder(data.projects, (p) => key(p.id), order) : []);
  const names = $derived(new Map(data?.projects.map((p) => [p.id, p.name]) ?? []));
  const disconnected = $derived(setup?.detected && (!setup.claudeFound || !setup.pluginEnabled));

  async function load() {
    try {
      const next = await api.overview();
      // On open, the service's order applies (attention and recent activity);
      // while browsing, the cards stay where they are.
      if (opened !== lastOpened || order.length === 0) {
        lastOpened = opened;
        order = next.projects.map((p) => key(p.id));
      }
      data = next;
    } catch (e) {
      failure(e);
    }
  }

  $effect(() => {
    void refresh;
    void opened;
    load();
  });

  // Already seen: leaves Waiting for you and the badge until the state changes.
  async function dismiss(card: ConversationCard) {
    try {
      await api.dismissAttention(card.id);
      await load();
    } catch (e) {
      failure(e);
    }
  }

  let searchTimer: ReturnType<typeof setTimeout>;
  $effect(() => {
    const q = query.trim();
    void refresh;
    clearTimeout(searchTimer);
    if (!q) {
      results = null;
      return;
    }
    searchTimer = setTimeout(async () => {
      try {
        results = await api.search(q, false);
      } catch (e) {
        failure(e);
      }
    }, 160);
  });
</script>

<header class="top">
  <div>
    <h1>Your conversations</h1>
    {#if data && data.totalConversations > 0}
      <p class="sub">
        {plural(projects.length, "project", "projects")} · {plural(data.totalConversations, "conversation", "conversations")}
      </p>
    {/if}
  </div>
  <button class="icon-btn" aria-label="Settings" onclick={() => go({ name: "settings" })}><Icon name="settings" /></button>
</header>

<div class="search">
  <Icon name="search" size={16} />
  <input
    class="field"
    type="search"
    placeholder="Search by topic or summary"
    aria-label="Search by topic or summary"
    bind:value={query}
  />
</div>

<div class="scroll">
  {#if disconnected}
    <section class="banner">
      <div>
        <strong>Connect your agents</strong>
        <p>{setup?.claudeFound ? "Claude Code isn't connected to Flow Keeper yet." : "Can't find Claude Code on this Mac."}</p>
      </div>
      <button class="btn small" onclick={() => go({ name: "onboarding" })}>Connect</button>
    </section>
  {/if}

  {#if results}
    <section class="list" aria-label="Search results">
      {#if results.length === 0}
        <p class="empty-line">No conversations match “{query.trim()}”.</p>
      {:else}
        {#each results as card (card.id)}
          <ConversationCardView {card} projectName={names.get(card.projectId) ?? "Unassigned"} />
        {/each}
      {/if}
    </section>
  {:else if data}
    {#if data.waiting.length}
      <section class="waiting" aria-label="Waiting for you">
        <h2>Waiting for you</h2>
        {#each data.waiting as card (card.id)}
          <div class="wait-row">
            <div class="wait-text">
              <strong>{card.title}</strong>
              <span>{names.get(card.projectId) ?? "Unassigned"} · {card.stateReason ?? "Needs your input"}</span>
            </div>
            <div class="wait-actions">
              <button class="btn small" onclick={() => primaryAction(card)}>{card.canResume ? "Resume" : "Open"}</button>
              <button class="icon-btn dismiss" aria-label="Seen, remove from Waiting for you" title="Seen" onclick={() => dismiss(card)}>
                <Icon name="close" size={14} />
              </button>
            </div>
          </div>
        {/each}
      </section>
    {/if}

    {#if projects.length}
      <section class="list" aria-label="Projects">
        {#each projects as project (key(project.id))}
          <ProjectCardView {project} onopen={() => go({ name: "project", id: project.id })} />
        {/each}
      </section>
    {:else}
      <section class="empty">
        <Robot mood="rest" size={120} animate={false} />
        <h2>No conversations yet</h2>
        <p>
          {disconnected
            ? "Connect Claude Code: from then on, conversations will show up here on their own."
            : "Start Claude Code in a terminal: the conversation will show up here on its own."}
        </p>
      </section>
    {/if}

    {#if data.hiddenCount}
      <button class="link" onclick={() => go({ name: "removed" })}>
        Removed from view ({data.hiddenCount})
      </button>
    {/if}
  {/if}
</div>

<style>
  .top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    padding: 20px 20px 10px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-3);
    font-size: 13px;
  }
  .search {
    position: relative;
    margin: 0 20px 12px;
    color: var(--text-3);
  }
  .search :global(svg) {
    position: absolute;
    left: 12px;
    top: 50%;
    transform: translateY(-50%);
  }
  .search .field {
    padding-left: 36px;
    border-radius: 999px;
    background: var(--surface);
    box-shadow: var(--shadow-card);
    border-color: transparent;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 20px 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 16px;
    border-radius: var(--radius-lg);
    background: var(--surface);
    border: 1px solid var(--line);
  }
  .banner p {
    margin: 2px 0 0;
    font-size: 13px;
    color: var(--text-2);
  }
  .waiting {
    background: var(--waiting-bg);
    border-radius: var(--radius-lg);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .waiting h2 {
    margin: 0;
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--waiting);
  }
  .wait-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .wait-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
  }
  .dismiss {
    width: 28px;
    height: 28px;
  }
  .wait-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .wait-text strong {
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wait-text span {
    font-size: 12.5px;
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: 40px 24px;
    gap: 6px;
  }
  .empty h2 {
    margin: 12px 0 0;
    font-size: 17px;
  }
  .empty p,
  .empty-line {
    margin: 0;
    color: var(--text-2);
    font-size: 13.5px;
    max-width: 300px;
  }
  .link {
    align-self: center;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-size: 13px;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
</style>
