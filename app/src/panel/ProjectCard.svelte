<script lang="ts">
  import Icon from "../lib/Icon.svelte";
  import { ago, countsSummary, plural, projectHue } from "../lib/format";
  import type { ProjectCard } from "../lib/types";

  let { project, onopen }: { project: ProjectCard; onopen: () => void } = $props();
  const hue = $derived(projectHue(project.name));
  const initial = $derived(project.id === null ? "?" : project.name.trim().charAt(0).toUpperCase());
</script>

<button
  class="card"
  class:unread={project.counts.unread > 0}
  onclick={onopen}
  aria-label={`${project.name}: ${countsSummary(project.counts)}${project.counts.unread ? " · new results to read" : ""}`}
>
  <span class="tile" style={`--h:${hue}`} aria-hidden="true">{initial}</span>
  <span class="text">
    <span class="name">{project.name}</span>
    {#if project.description}<span class="desc">{project.description}</span>{/if}
    <span class="status">
      {#if project.counts.waiting}<span class="dot waiting" aria-hidden="true"></span>{/if}
      {#if project.counts.working}<span class="dot working" aria-hidden="true"></span>{/if}
      {#if project.counts.unread}<span class="dot finished" aria-hidden="true"></span>{/if}
      <span>{countsSummary(project.counts)}</span>
    </span>
    <span class="foot">{plural(project.counts.total, "conversation", "conversations")} · {ago(project.lastActivityAt)}</span>
  </span>
  <span class="chev"><Icon name="chevron" /></span>
</button>

<style>
  .card {
    width: 100%;
    border: 0;
    text-align: left;
    display: grid;
    grid-template-columns: 52px 1fr 20px;
    gap: 14px;
    align-items: center;
    padding: 14px 14px 14px 14px;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    transition: transform 140ms ease, box-shadow 140ms ease;
  }
  .card:hover {
    transform: translateY(-1px);
    box-shadow: var(--shadow-pop);
  }
  /* finished turns not read yet: until you open the project or work resumes */
  .card.unread {
    box-shadow: 0 0 0 2px var(--finished), var(--shadow-card);
  }
  .card.unread:hover {
    box-shadow: 0 0 0 2px var(--finished), var(--shadow-pop);
  }
  .tile {
    width: 52px;
    height: 52px;
    border-radius: 14px;
    display: grid;
    place-items: center;
    font-size: 21px;
    font-weight: 700;
    background: hsl(var(--h) 70% 92%);
    color: hsl(var(--h) 45% 32%);
  }
  @media (prefers-color-scheme: dark) {
    .tile {
      background: hsl(var(--h) 30% 24%);
      color: hsl(var(--h) 60% 82%);
    }
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name {
    font-size: 16px;
    font-weight: 650;
    letter-spacing: -0.01em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .desc {
    color: var(--text-2);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    font-weight: 560;
    color: var(--text);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
  }
  .dot.waiting {
    background: var(--waiting);
  }
  .dot.working {
    background: var(--working);
  }
  .dot.finished {
    background: var(--finished);
  }
  .foot {
    font-size: 12px;
    color: var(--text-3);
  }
  .chev {
    color: var(--text-3);
  }
</style>
