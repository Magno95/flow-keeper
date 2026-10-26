<script lang="ts" module>
  export type MenuItem = { label: string; hint?: string; danger?: boolean; run: () => void };
</script>

<script lang="ts">
  import Icon from "../lib/Icon.svelte";

  let { items, label = "More actions" }: { items: MenuItem[]; label?: string } = $props();

  let open = $state(false);
  let up = $state(false);
  let root: HTMLDivElement;
  let button: HTMLButtonElement;

  function toggle() {
    if (!open) {
      const rect = button.getBoundingClientRect();
      up = window.innerHeight - rect.bottom < 40 + items.length * 52;
    }
    open = !open;
  }
  function pick(item: MenuItem) {
    open = false;
    item.run();
  }
  function outside(e: MouseEvent) {
    if (open && !root.contains(e.target as Node)) open = false;
  }
  function keys(e: KeyboardEvent) {
    if (open && e.key === "Escape") {
      e.stopPropagation();
      open = false;
      button.focus();
    }
  }
</script>

<svelte:window onclick={outside} onkeydown={keys} />

<div class="menu" bind:this={root}>
  <button bind:this={button} class="icon-btn" aria-label={label} aria-haspopup="menu" aria-expanded={open} onclick={toggle}>
    <Icon name="more" />
  </button>
  {#if open}
    <div class="list" class:up role="menu">
      {#each items as item}
        <button role="menuitem" class="item" class:danger={item.danger} onclick={() => pick(item)}>
          <span>{item.label}</span>
          {#if item.hint}<small>{item.hint}</small>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
  }
  .list {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 20;
    min-width: 230px;
    padding: 6px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-pop);
  }
  .list.up {
    top: auto;
    bottom: calc(100% + 6px);
  }
  .item {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 8px 10px;
    border-radius: 8px;
    font-size: 13.5px;
  }
  .item:hover,
  .item:focus-visible {
    background: var(--surface-muted);
  }
  .item small {
    color: var(--text-3);
    font-size: 12px;
  }
  .danger span {
    color: var(--danger);
  }
</style>
