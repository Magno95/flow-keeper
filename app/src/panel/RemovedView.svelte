<script lang="ts">
  import { api } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import { failure } from "../lib/ui.svelte";
  import type { ConversationCard } from "../lib/types";
  import ConversationCardView from "./ConversationCard.svelte";
  import type { Route } from "./routes";

  let { refresh, go }: { refresh: number; go: (r: Route) => void } = $props();
  let cards = $state<ConversationCard[]>([]);

  $effect(() => {
    void refresh;
    api.removed().then((c) => (cards = c), failure);
  });
</script>

<header class="top">
  <button class="back" onclick={() => go({ name: "overview" })}><Icon name="back" size={16} /> Projects</button>
</header>
<div class="title">
  <h1>Removed from view</h1>
  <p class="sub">They stay here until you restore them. If a conversation really picks up again, it goes back to its project on its own.</p>
</div>
<div class="scroll">
  {#each cards as card (card.id)}
    <ConversationCardView {card} />
  {:else}
    <p class="sub">No removed conversations.</p>
  {/each}
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
  .back:hover {
    background: var(--surface-muted);
  }
  .title {
    padding: 6px 20px 12px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .sub {
    margin: 4px 0 0;
    color: var(--text-2);
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
