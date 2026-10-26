<script lang="ts">
  import { isTauri } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { api, listen, onCompanion, onUpdate } from "../lib/api";
  import Robot, { bubbleRise, robotHeight, type Gaze } from "../lib/Robot.svelte";
  import type { CompanionState } from "../lib/types";

  const SIZE = 92;
  // Base window 112×96 (tauri.conf.json): robot centered below a 4 px margin.
  const WINDOW_H = 96;
  const PAD_TOP = 4;
  // Distance between the robot's bottom and the top edge of the base window.
  const ROBOT_BOTTOM = PAD_TOP + (WINDOW_H - PAD_TOP + robotHeight(SIZE)) / 2;
  // Space to add on top so the bubble doesn't get clipped.
  const BUBBLE_HEADROOM = Math.max(0, Math.ceil(bubbleRise(SIZE) - ROBOT_BOTTOM) + 2);

  let cs = $state<CompanionState>({ mood: "rest", waiting: 0, working: 0 });
  // “New version” bubble: stays until you click the companion.
  let bubble = $state(false);
  let gaze = $state<Gaze>(null);
  let finishedTick = $state(0);
  // A turn finished while the overview was closed: the "!" stays until it is opened.
  let unseenFinish = $state(false);
  let panelOpen = false;
  let winkTick = $state(0);
  let notifyTick = $state(0);
  // The robot starts from the real state, so the notice doesn't fire at launch.
  let ready = $state(false);
  let start: { x: number; y: number } | null = null;
  let dragging = false;
  let lastWinkAt = 0;

  const label = $derived(
    cs.mood === "attention"
      ? cs.waiting === 1
        ? "One conversation is waiting for you"
        : `${cs.waiting} conversations are waiting for you`
      : bubble
        ? "A new version is available"
        : unseenFinish
          ? "A conversation has finished"
          : cs.mood === "problem"
        ? "Flow Keeper connection problem"
        : cs.mood === "working"
          ? cs.working === 1
            ? "One agent working"
            : `${cs.working} agents working`
          : "No conversation needs your attention",
  );

  // One more conversation in the same state: notice without a mood change
  // (a state change already has its own notice in Robot).
  function update(s: CompanionState) {
    if (s.mood === cs.mood && (s.working > cs.working || s.waiting > cs.waiting)) notifyTick++;
    cs = s;
  }

  onMount(() => {
    Promise.allSettled([
      api.companion().then((s) => s && (cs = s)),
      api.update().then((u) => (bubble = u.announce)),
    ]).finally(() => (ready = true));
    const subs = [
      onCompanion(update),
      onUpdate((u) => (bubble = u.announce)),
      listen<Gaze>("tracker://panel-opened", (e) => {
        gaze = e.payload ?? null;
        panelOpen = true;
        unseenFinish = false;
      }),
      listen("tracker://panel-closed", () => {
        gaze = null;
        panelOpen = false;
      }),
      listen("tracker://turn-finished", () => {
        if (!panelOpen) unseenFinish = true;
        finishedTick++;
      }),
    ];
    return () => subs.forEach((p) => p.then((f) => f()));
  });

  // The window grows upward for the bubble and goes back to normal when it disappears.
  $effect(() => {
    api.setCompanionHeadroom(bubble ? BUBBLE_HEADROOM : 0).catch(() => {});
  });

  // A click opens the overview (with a wink); a drag moves the companion.
  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    start = { x: e.screenX, y: e.screenY };
    dragging = false;
  }
  function move(e: PointerEvent) {
    if (!start || dragging) return;
    if (Math.abs(e.screenX - start.x) + Math.abs(e.screenY - start.y) > 4) {
      dragging = true;
      if (isTauri()) getCurrentWindow().startDragging();
    }
  }
  function up() {
    if (start && !dragging) open();
    start = null;
  }
  // Wink on hover; the click right after doesn't repeat it.
  function wink() {
    winkTick++;
    lastWinkAt = Date.now();
  }
  function enter() {
    if (!start) wink();
  }
  function open() {
    if (Date.now() - lastWinkAt > 900) wink();
    if (bubble) {
      // Clicking the bubble dismisses it for this version and goes to the update.
      bubble = false;
      api.dismissUpdate();
      api.openUpdates();
    } else api.togglePanel();
  }
  function key(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open();
    }
  }
</script>

<button
  class="companion"
  aria-label={`FloK: ${label}. ${bubble ? "Open the update" : "Open the overview"}`}
  title={label}
  onpointerenter={enter}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onkeydown={key}
>
  <span class="body">
    {#if ready}
      <Robot mood={cs.mood} {gaze} {finishedTick} {winkTick} {notifyTick} {bubble} exclaim={unseenFinish} size={SIZE} />
    {/if}
    {#if cs.working > 0}
      <span class="badge working" aria-hidden="true">
        <span class="count">{cs.working > 9 ? "9+" : cs.working}</span>
      </span>
    {/if}
    {#if cs.waiting > 0}
      <span class="badge" aria-hidden="true">{cs.waiting > 9 ? "9+" : cs.waiting}</span>
    {/if}
  </span>
</button>

<style>
  :global(html),
  :global(body) {
    background: transparent !important;
  }
  .companion {
    position: relative;
    width: 100%;
    height: 100%;
    border: 0;
    padding: 0;
    background: transparent;
    /* the robot stays at the bottom: the bubble space is added on top */
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    cursor: pointer;
    border-radius: 20px;
  }
  .body {
    position: relative;
    flex: none;
    height: 96px;
    display: grid;
    place-items: center;
    /* room for the "!" sticking out above the head */
    padding-top: 4px;
  }
  .companion:focus-visible {
    outline-offset: -3px;
  }
  /* bottom right: the "!" of new notices sticks out at the top */
  .badge {
    position: absolute;
    bottom: 4px;
    right: 2px;
    min-width: 22px;
    height: 22px;
    padding: 0 6px;
    border-radius: 999px;
    background: #e8590c;
    color: #fff;
    font-size: 12.5px;
    font-weight: 700;
    display: grid;
    place-items: center;
    box-shadow: 0 0 0 2px #fff, 0 2px 6px rgba(0, 0, 0, 0.25);
  }
  /* bottom left, mirroring the waiting one: a ring that keeps turning */
  .badge.working {
    right: auto;
    left: 2px;
    padding: 0;
    width: 24px;
    height: 24px;
    min-width: 0;
    overflow: hidden;
    background: #fff;
    isolation: isolate;
  }
  .badge.working::before {
    content: "";
    position: absolute;
    inset: -50%;
    z-index: -1;
    background: conic-gradient(from 0deg, transparent 0 30%, #2f6fed);
    animation: spin 1.1s linear infinite;
  }
  .badge.working .count {
    position: absolute;
    inset: 3px;
    border-radius: inherit;
    background: #2f6fed;
    box-shadow: 0 0 0 1px #fff;
    display: grid;
    place-items: center;
  }
  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .badge.working::before {
      animation: none;
    }
  }
</style>
