<script lang="ts">
  import { api } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import { countsSummary, stableOrder } from "../lib/format";
  import { failure, toast, ui } from "../lib/ui.svelte";
  import type { ProjectDetail } from "../lib/types";
  import ConversationCardView from "./ConversationCard.svelte";
  import Menu, { type MenuItem } from "./Menu.svelte";
  import type { Route } from "./routes";

  let { id, refresh, opened, go }: { id: number | null; refresh: number; opened: number; go: (r: Route) => void } = $props();

  let data = $state<ProjectDetail | null>(null);
  let order: number[] = [];
  let lastOpened = -1;
  // Results still unread on arrival, with the activity they had: they stay
  // highlighted while you are here and are marked read right away. Any new
  // activity (you went on in the terminal) clears the highlight.
  let fresh = $state(new Map<number, number>());
  const cards = $derived(data ? stableOrder(data.conversations, (c) => c.id, order) : []);

  async function load() {
    try {
      const next = await api.project(id);
      if (!next) return go({ name: "overview" });
      if (opened !== lastOpened || order.length === 0) {
        lastOpened = opened;
        order = next.conversations.map((c) => c.id);
      }
      data = next;
      const unread = next.conversations.filter((c) => c.unread);
      if (unread.length) {
        fresh = new Map([...fresh, ...unread.map((c) => [c.id, c.lastActivityAt] as const)]);
        await api.markProjectRead(id);
      }
      if (next.conversations.length === 0) go({ name: "overview" });
    } catch (e) {
      failure(e);
    }
  }

  $effect(() => {
    void refresh;
    void opened;
    load();
  });

  const items = $derived<MenuItem[]>(
    data && data.project.id !== null
      ? [
          {
            label: "Edit name and description",
            run: () =>
              (ui.dialog = { kind: "project", id: data!.project.id!, name: data!.project.name, description: data!.project.description }),
          },
          data.project.recapExcluded
            ? {
                label: "Include in AI summaries",
                run: async () => {
                  await api.setProjectExcluded(data!.project.id!, false);
                  toast("Summaries for this project will resume at the end of the next turn.");
                },
              }
            : {
                label: "Exclude from AI summaries",
                hint: "No content from this project will be sent.",
                run: async () => {
                  await api.setProjectExcluded(data!.project.id!, true);
                  toast("Project excluded: status, source and manual titles are kept.");
                },
              },
          { label: "Merge with another project", run: () => (ui.dialog = { kind: "merge", id: data!.project.id!, name: data!.project.name }) },
        ]
      : [],
  );
</script>

<header class="top">
  <button class="back" onclick={() => go({ name: "overview" })}><Icon name="back" size={16} /> Projects</button>
  {#if items.length}<Menu {items} label="Project actions" />{/if}
</header>

{#if data}
  <div class="title">
    <h1>{data.project.name}</h1>
    {#if data.project.description}<p class="desc">{data.project.description}</p>{/if}
    <p class="sub">
      {countsSummary(data.project.counts)}{data.project.recapExcluded ? " · excluded from summaries" : ""}
    </p>
  </div>
  <div class="scroll">
    {#each cards as card (card.id)}
      <ConversationCardView {card} highlight={fresh.get(card.id) === card.lastActivityAt} />
    {/each}
  </div>
{/if}

<style>
  .top {
    display: flex;
    justify-content: space-between;
    align-items: center;
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
  .back:hover {
    background: var(--surface-muted);
    color: var(--text);
  }
  .title {
    padding: 6px 20px 12px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    overflow-wrap: anywhere;
  }
  .desc {
    margin: 2px 0 0;
    color: var(--text-2);
    font-size: 13.5px;
  }
  .sub {
    margin: 4px 0 0;
    color: var(--text-3);
    font-size: 13px;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 2px 20px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
</style>
