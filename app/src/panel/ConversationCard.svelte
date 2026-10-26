<script lang="ts">
  import { api } from "../lib/api";
  import { ago, dateTime } from "../lib/format";
  import { primaryAction, removeFromView, restore, ui } from "../lib/ui.svelte";
  import type { ConversationCard } from "../lib/types";
  import Menu, { type MenuItem } from "./Menu.svelte";
  import StatusPill from "./StatusPill.svelte";

  let { card, projectName = null }: { card: ConversationCard; projectName?: string | null } = $props();
  let expanded = $state(false);

  const placeholder = $derived.by(() => {
    if (card.summary) return null;
    switch (card.summaryStatus) {
      case "disabled":
        return "AI summaries not set up";
      case "excluded":
        return "Project excluded from summaries";
      case "budget":
        return "Summary paused: budget reached";
      case "failed":
        return "Summary unavailable";
      case "waiting":
        return "The summary arrives when the agent finishes a turn";
      case "ready":
        return "Nothing to summarize yet";
      default:
        return "Summary in progress";
    }
  });
  const lateNote = $derived(
    card.summary && ["stale", "retry", "budget"].includes(card.summaryStatus)
      ? "Summary needs updating"
      : card.summary && card.summaryStatus === "failed"
        ? "Summary not updated"
        : null,
  );
  const primary = $derived(card.canResume ? "Resume" : "Open conversation");

  // Agents and editors recognizable at a glance by their brand color.
  const brands: Record<string, string> = { Claude: "claude", Codex: "codex", "VS Code": "vscode", iTerm: "iterm" };

  const items = $derived<MenuItem[]>(
    card.hidden
      ? [{ label: "Restore to view", run: () => restore(card) }]
      : [
          { label: "Edit title", run: () => (ui.dialog = { kind: "rename", card }) },
          { label: "Move to another project", run: () => (ui.dialog = { kind: "move", card }) },
          { label: "Update summary", run: () => api.requestRecap(card.id) },
          {
            label: "Remove from view",
            hint: "Doesn't close the agent. It reappears if activity resumes.",
            danger: true,
            run: () => removeFromView(card),
          },
        ],
  );
</script>

<article class="card {card.state}" aria-label={card.title}>
  <header>
    <StatusPill state={card.state} label={card.stateLabel} />
    <time datetime={new Date(card.lastActivityAt).toISOString()} title={dateTime(card.lastActivityAt)}>
      {ago(card.lastActivityAt)}
    </time>
  </header>

  <button class="body" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
    <h3 class:provisional={card.titleIsProvisional}>{card.title}</h3>
    {#if card.summary}
      <p class="summary">{card.summary}</p>
    {:else}
      <p class="summary muted">{placeholder}</p>
    {/if}
    {#if lateNote}<p class="late">{lateNote}</p>{/if}
    {#if card.state === "waiting" && card.stateReason}
      <p class="reason">{card.stateReason}</p>
    {/if}
  </button>

  {#if expanded}
    <dl class="details">
      {#if card.stateReason && card.state !== "waiting"}
        <div><dt>Status</dt><dd>{card.stateReason}</dd></div>
      {/if}
      <div><dt>Last activity</dt><dd>{dateTime(card.lastActivityAt)}</dd></div>
      <div>
        <dt>Summary</dt>
        <dd>{card.summaryAt ? `updated ${dateTime(card.summaryAt)}` : "not generated yet"}</dd>
      </div>
      <div>
        <dt>Environment</dt>
        <dd>
          {card.environment}{card.environmentIsGuess ? " (not identified for certain: the session was started from an unrecognized terminal)" : ""}
        </dd>
      </div>
      {#if card.titleIsProvisional}
        <div><dt>Title</dt><dd>provisional, waiting for the first summary</dd></div>
      {/if}
    </dl>
  {/if}

  <footer>
    <span class="meta">
      {#each [card.agent, card.environment] as name, i}
        {#if brands[name]}<span class="brand {brands[name]}">{name}</span>{:else}<span>{name}</span>{/if}
        {#if i === 0 && !brands[card.agent] && !brands[card.environment]}<span aria-hidden="true">·</span>{/if}
      {/each}
      {#if card.folderLabel}<span class="chip">{card.folderLabel}</span>{/if}
      {#if projectName}<span class="chip">{projectName}</span>{/if}
    </span>
    <span class="actions">
      {#if card.hidden}
        <button class="btn small secondary" onclick={() => restore(card)}>Restore</button>
      {:else}
        <button class="btn small" disabled={ui.busy[card.id]} onclick={() => primaryAction(card)}>{primary}</button>
      {/if}
      <Menu {items} label={`More actions for ${card.title}`} />
    </span>
  </footer>
</article>

<style>
  .card {
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    padding: 14px 14px 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    transition: opacity 160ms ease;
  }
  .card.closed {
    background: color-mix(in srgb, var(--surface) 60%, var(--bg));
    box-shadow: none;
    border: 1px solid var(--line);
  }
  .card.closed h3,
  .card.closed .summary {
    color: var(--text-2);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  time {
    color: var(--text-3);
    font-size: 12px;
  }
  .body {
    border: 0;
    background: transparent;
    text-align: left;
    padding: 2px 0;
    border-radius: 8px;
    cursor: default;
  }
  h3 {
    margin: 0 0 3px;
    font-size: 15.5px;
    font-weight: 650;
    letter-spacing: -0.01em;
    line-height: 1.3;
    overflow-wrap: anywhere;
  }
  h3.provisional {
    font-weight: 560;
  }
  .summary {
    margin: 0;
    color: var(--text-2);
    font-size: 13.5px;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .summary.muted {
    color: var(--text-3);
    font-style: italic;
  }
  .late {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-3);
  }
  .reason {
    margin: 6px 0 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--waiting);
  }
  .details {
    margin: 2px 0 4px;
    padding: 10px 12px;
    background: var(--surface-muted);
    border-radius: var(--radius-sm);
    font-size: 12.5px;
    display: grid;
    gap: 4px;
  }
  .details div {
    display: grid;
    grid-template-columns: 104px 1fr;
    gap: 8px;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    color: var(--text-2);
  }
  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .meta {
    color: var(--text-3);
    font-size: 12.5px;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .chip {
    background: var(--surface-muted);
    color: var(--text-2);
    border-radius: 6px;
    padding: 1px 7px;
    font-size: 11.5px;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .brand {
    color: #fff;
    border-radius: 6px;
    padding: 1px 7px;
    font-size: 11.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  .brand.claude {
    background: #d97757;
  }
  .brand.codex {
    background: #6e56f8;
  }
  .brand.vscode {
    background: #007acc;
  }
  .brand.iterm {
    background: #000;
    color: #3fe04f;
  }
  @media (prefers-color-scheme: dark) {
    .brand.iterm {
      box-shadow: inset 0 0 0 1px rgb(63 224 79 / 35%);
    }
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
</style>
