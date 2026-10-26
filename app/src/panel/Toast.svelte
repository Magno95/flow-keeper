<script lang="ts">
  import { dismissToast, holdToast, ui } from "../lib/ui.svelte";

  // Past this horizontal distance, releasing the drag dismisses the notice.
  const DISMISS_AT = 72;
  const LEAVE_MS = 160;

  let dx = $state(0);
  let dragging = $state(false);
  let leaving = $state(0);
  let startX = 0;

  // A new notice starts from the middle, even if the previous one was being dragged.
  $effect(() => {
    ui.toast?.id;
    dx = 0;
    dragging = false;
    leaving = 0;
  });

  function leave(direction: number) {
    const id = ui.toast?.id;
    leaving = direction;
    holdToast();
    setTimeout(() => ui.toast?.id === id && dismissToast(), LEAVE_MS);
  }

  function down(e: PointerEvent) {
    if (e.button !== 0 || leaving || (e.target as Element).closest("button")) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    startX = e.clientX;
    dragging = true;
    holdToast();
  }

  function move(e: PointerEvent) {
    if (dragging) dx = e.clientX - startX;
  }

  function up() {
    if (!dragging) return;
    dragging = false;
    if (Math.abs(dx) >= DISMISS_AT) {
      leave(Math.sign(dx));
    } else {
      dx = 0;
      holdToast(3000);
    }
  }
</script>

<div class="slot" aria-live="polite">
  {#if ui.toast}
    {@const t = ui.toast}
    <div
      class="toast"
      class:dragging
      role="status"
      style:transform={leaving ? `translateX(${leaving * 120}%)` : `translateX(${dx}px)`}
      style:opacity={leaving ? 0 : 1 - Math.min(Math.abs(dx) / (DISMISS_AT * 2.5), 0.6)}
      style:transition-duration={dragging ? "0ms" : `${LEAVE_MS}ms`}
      onpointerdown={down}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={up}
    >
      <span>{t.message}</span>
      <div class="actions">
        {#if t.action}
          <button
            class="action"
            onclick={() => {
              t.action!.run();
              dismissToast();
            }}>{t.action.label}</button
          >
        {/if}
        <button class="close" aria-label="Close" onclick={() => leave(1)}>
          <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
            <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .slot {
    position: absolute;
    left: 16px;
    right: 16px;
    bottom: 16px;
    z-index: 50;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    background: #1f1f1f;
    color: #fff;
    border-radius: var(--radius-md);
    padding: 11px 8px 11px 14px;
    font-size: 13px;
    box-shadow: var(--shadow-pop);
    cursor: grab;
    user-select: none;
    touch-action: pan-y;
    transition-property: transform, opacity;
    transition-timing-function: ease-out;
  }
  .toast.dragging {
    cursor: grabbing;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  button {
    border: 0;
    background: transparent;
    cursor: pointer;
  }
  .action {
    color: #ffd2b0;
    font-weight: 700;
    white-space: nowrap;
  }
  .close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border-radius: 50%;
    color: rgba(255, 255, 255, 0.6);
  }
  .close:hover {
    color: #fff;
    background: rgba(255, 255, 255, 0.12);
  }
</style>
