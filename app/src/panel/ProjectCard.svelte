<script lang="ts">
  import Icon from "../lib/Icon.svelte";
  import { ago, countsSummary, plural, projectHue } from "../lib/format";
  import type { ProjectCard } from "../lib/types";

  let {
    project,
    onopen,
    onremove,
    revealed,
    onreveal,
  }: { project: ProjectCard; onopen: () => void; onremove: () => void; revealed: boolean; onreveal: (open: boolean) => void } =
    $props();
  const hue = $derived(projectHue(project.name));
  const initial = $derived(project.id === null ? "?" : project.name.trim().charAt(0).toUpperCase());

  // Dragged to the left (pointer or two-finger swipe), the card uncovers the
  // button that removes the project from view.
  const REVEAL = 64;
  let dragging = $state(false);
  let offset = $state(0);
  const shift = $derived(dragging ? offset : revealed ? -REVEAL : 0);
  const clamp = (x: number) => Math.max(-REVEAL * 1.5, Math.min(0, x));

  let pointer: number | null = null;
  let startX = 0;
  let startY = 0;
  let moved = false;

  function begin() {
    dragging = true;
    offset = revealed ? -REVEAL : 0;
  }

  function settle() {
    dragging = false;
    onreveal(offset < -REVEAL / 2);
  }

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    pointer = e.pointerId;
    startX = e.clientX;
    startY = e.clientY;
    moved = false;
  }

  function move(e: PointerEvent) {
    if (pointer !== e.pointerId) return;
    const dx = e.clientX - startX;
    if (!dragging) {
      const dy = e.clientY - startY;
      if (Math.abs(dx) < 6 && Math.abs(dy) < 6) return;
      if (Math.abs(dx) < Math.abs(dy)) {
        pointer = null;
        return;
      }
      begin();
      moved = true;
      (e.currentTarget as Element).setPointerCapture(e.pointerId);
    }
    offset = clamp((revealed ? -REVEAL : 0) + dx);
  }

  function up() {
    pointer = null;
    if (!dragging) return;
    settle();
    // The click that follows the drag must not open the project.
    setTimeout(() => (moved = false));
  }

  let wheelTimer: ReturnType<typeof setTimeout>;
  function wheel(e: WheelEvent) {
    if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    if (!dragging) begin();
    offset = clamp(offset - e.deltaX);
    clearTimeout(wheelTimer);
    wheelTimer = setTimeout(settle, 120);
  }

  function click() {
    if (moved) return;
    if (revealed) onreveal(false);
    else onopen();
  }
</script>

<div class="row" onwheel={wheel}>
  <button
    class="remove"
    style:opacity={Math.min(1, -shift / REVEAL)}
    tabindex={revealed ? 0 : -1}
    aria-hidden={!revealed}
    aria-label={`Remove ${project.name} from view`}
    title="Remove from view. It reappears if activity resumes."
    onclick={onremove}
  >
    <Icon name="close" size={16} />
  </button>
  <button
    class="card"
    class:unread={project.counts.unread > 0}
    class:dragging
    style:transform={shift ? `translateX(${shift}px)` : null}
    onclick={click}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
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
</div>

<style>
  .row {
    position: relative;
  }
  .remove {
    position: absolute;
    top: 50%;
    right: 12px;
    width: 40px;
    height: 40px;
    margin-top: -20px;
    border: 0;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--danger);
    color: #fff;
    cursor: pointer;
  }
  .remove[aria-hidden="true"] {
    pointer-events: none;
  }
  .card {
    position: relative;
    width: 100%;
    touch-action: pan-y;
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
  .card.dragging {
    transition: none;
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
