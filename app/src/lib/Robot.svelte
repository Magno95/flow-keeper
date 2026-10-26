<script lang="ts" module>
  import manifest from "../assets/companion/sprites.json";

  const urls = import.meta.glob("../assets/companion/*.png", { eager: true, query: "?url", import: "default" }) as Record<
    string,
    string
  >;
  const frameUrl = (id: string) => urls[`../assets/companion/${id}.png`];

  type Seq = { frames: string[]; durations: number[] | null };
  const seqs = manifest.sequences as Record<string, Seq>;
  const frameInfo = manifest.frames as Record<string, { width: number; height: number }>;
  // Speech bubbles: layers above the frame, bottom-aligned; `left` relative to the frame.
  const overlays = manifest.overlays as Record<string, { width: number; height: number; left: number }>;
  // Head proportion: the taller frames (the "!") stick out above.
  const BASE = frameInfo["rest-1"];

  export const robotHeight = (size: number) => (size * BASE.height) / BASE.width;
  /** How far the bubble rises above the robot's bottom, in px, at a given width. */
  export const bubbleRise = (size: number) => (overlays.bubble.height * size) / BASE.width;

  export type Gaze = { vertical: "up" | "down"; side: "left" | "right" | "center" } | null;
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import type { Mood } from "./types";

  let {
    mood = "rest",
    gaze = null,
    finishedTick = 0,
    winkTick = 0,
    notifyTick = 0,
    bubble = false,
    exclaim = false,
    size = 92,
    animate = true,
  }: {
    mood?: Mood;
    gaze?: Gaze;
    finishedTick?: number;
    winkTick?: number;
    notifyTick?: number;
    /** Update bubble: stays above the robot whatever the state. */
    bubble?: boolean;
    /** Unseen finished turn: the "!" stays up until the overview is opened. */
    exclaim?: boolean;
    size?: number;
    animate?: boolean;
  } = $props();

  // Hold poses: tell the states apart even without movement.
  const HOLD: Record<Mood, string> = {
    rest: "rest-1",
    working: "working-3",
    attention: "waiting-6",
    problem: "problem-6",
  };
  const ENTRY: Record<Mood, string | null> = { rest: null, working: "genie", attention: "waiting", problem: "problem" };

  // Bubble stage while it appears; otherwise it is full.
  type Stage = "none" | "small" | "full";
  // `null`: the step changes only the bubble and leaves the face as it is.
  type Step = [frame: string | null, ms: number, stage?: Stage];
  const steps = (name: string): Step[] => {
    const seq = seqs[name];
    return seq?.durations ? seq.frames.map((f, i) => [f, seq.durations![i]]) : [];
  };
  const BUBBLE_STAGES: Stage[] = ["none", "small", "full"];
  // Sequences that change the look end with an intermediate step,
  // so the glasses and focused gaze don't vanish abruptly.
  const SCENES: Record<string, () => Step[]> = {
    working: () => steps("working"),
    genie: () => [...steps("genie"), ["genie-2", 140]],
    intense: () => [...steps("intense"), ["intense-2", 220]],
    rest: () => steps("rest"),
    waiting: () => steps("waiting"),
    problem: () => steps("problem"),
    finished: () => steps("finished"),
    // After the smile the "!" pops up and stays there (pose EXCLAIM_HOLD).
    notice: () => [...steps("finished"), ...steps("new").slice(0, 4)],
    wink: () => steps("wink"),
    new: () => steps("new"),
    // The bubble pops up small and then fills out, while the robot looks at it and smiles.
    update: () => steps("update").map(([f, ms], i) => [f, ms, BUBBLE_STAGES[Math.min(i, 2)]]),
    // Only the bubble, when the face must stay on attention or on the panel.
    bubble: () => [[null, 160, "none"], [null, 160, "small"]],
  };
  // The new-conversation "!" takes the same space as the bubble.
  const EXCLAIM = new Set(seqs.new.frames);
  const EXCLAIM_HOLD = "new-4";
  // Working movements: alternating between an exploring gaze and glasses, with
  // the occasional intense focus.
  const WORKING_IDLE: [string, number][] = [
    ["genie", 0.42],
    ["working", 0.4],
    ["intense", 0.18],
  ];
  function pickWorking(): string {
    let r = Math.random();
    for (const [name, w] of WORKING_IDLE) if ((r -= w) < 0) return name;
    return WORKING_IDLE[0][0];
  }

  let frame = $state(HOLD.rest);
  let stage = $state<Stage>("full");
  const overlay = $derived(bubble && stage !== "none" && !EXCLAIM.has(frame) ? (stage === "small" ? "bubble-small" : "bubble") : null);
  let timers: ReturnType<typeof setTimeout>[] = [];
  // The finish notice is running: going back to rest doesn't cut it short.
  let noticing = false;
  let idle: ReturnType<typeof setTimeout> | null = null;
  const reduced = typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const still = $derived(reduced || !animate);

  function gazeFrames(g: NonNullable<Gaze>): string[] {
    const sheet = g.vertical;
    if (g.side === "right") return [`${sheet}-2`, `${sheet}-3`];
    if (g.side === "left") return [`${sheet}-5`, `${sheet}-6`];
    return [`${sheet}-1`];
  }

  // An attention request takes precedence over plain visual exploration;
  // the update bubble over the "!" (they take the same space).
  const holding = $derived(exclaim && mood !== "attention" && !bubble);
  const restingPose = $derived(
    gaze && mood !== "attention" ? gazeFrames(gaze).at(-1)! : holding ? EXCLAIM_HOLD : HOLD[mood],
  );

  function stop() {
    timers.forEach(clearTimeout);
    timers = [];
    noticing = false;
    stage = "full";
  }

  /** One or more scenes in a row, then the pose returned by `after`. */
  function play(names: string | string[], after: () => string) {
    stop();
    const all = [names].flat().flatMap((n) => SCENES[n]?.() ?? []);
    if (still || !all.length) {
      frame = after();
      return;
    }
    let t = 0;
    for (const [f, ms, st] of all) {
      timers.push(
        setTimeout(() => {
          if (f) frame = f;
          if (st) stage = st;
        }, t),
      );
      t += ms;
    }
    timers.push(
      setTimeout(() => {
        frame = after();
        stage = "full";
        noticing = false;
      }, t),
    );
  }

  /** Occasional movements, never a continuous loop: spaced-out blinks or glances. */
  function scheduleIdle() {
    if (idle) clearTimeout(idle);
    idle = null;
    if (still || gaze || holding) return;
    const [min, max] = mood === "rest" ? [5000, 11000] : mood === "working" ? [12000, 22000] : [0, 0];
    if (!max) return;
    idle = setTimeout(() => {
      play(mood === "rest" ? "rest" : pickWorking(), () => restingPose);
      scheduleIdle();
    }, min + Math.random() * (max - min));
  }

  // Entering a new state: the "!" as a notice (not on first launch nor
  // when going back to rest), the state's sequence once, then the pose.
  let lastMood: Mood | null = null;
  $effect(() => {
    if (mood === lastMood) return;
    const first = lastMood === null;
    lastMood = mood;
    const entry = ENTRY[mood];
    if (entry && !(gaze && mood !== "attention")) play(first ? entry : ["new", entry], () => restingPose);
    else if (!noticing) {
      stop();
      frame = restingPose;
    }
    scheduleIdle();
  });

  // Gaze toward the panel: intermediate pose ~120 ms, then held.
  let lastGaze = "";
  $effect(() => {
    const key = JSON.stringify(gaze);
    if (key === lastGaze) return;
    lastGaze = key;
    stop();
    if (gaze && mood !== "attention") {
      const [first, final] = gazeFrames(gaze);
      if (final && !still) {
        frame = first;
        timers.push(setTimeout(() => (frame = final), 120));
      } else frame = final ?? first;
    } else frame = restingPose;
    scheduleIdle();
  });

  let lastFinished = 0;
  $effect(() => {
    if (finishedTick === lastFinished) return;
    lastFinished = finishedTick;
    // Finishing doesn't hide an attention request.
    if (mood !== "attention" && !gaze) {
      play(exclaim ? "notice" : "finished", () => restingPose);
      noticing = exclaim && !still;
    }
  });

  // "!" seen (overview opened without the gaze changing): back to the state's pose.
  let lastExclaim = false;
  $effect(() => {
    if (exclaim === lastExclaim) return;
    lastExclaim = exclaim;
    if (!exclaim && !noticing && frame === EXCLAIM_HOLD) frame = restingPose;
    scheduleIdle();
  });

  // New conversation without a state change: just the notice, then the current state.
  let lastNotify = 0;
  $effect(() => {
    if (notifyTick === lastNotify) return;
    lastNotify = notifyTick;
    if (gaze && mood !== "attention") return;
    const entry = mood === "attention" ? "waiting" : null;
    play(entry ? ["new", entry] : "new", () => restingPose);
  });

  // Bubble appearance (not at launch): with its face free, the robot looks at it and smiles.
  let lastBubble: boolean | null = null;
  $effect(() => {
    if (bubble === lastBubble) return;
    const first = lastBubble === null;
    lastBubble = bubble;
    if (!bubble || first) return;
    play(mood === "attention" || gaze ? "bubble" : "update", () => restingPose);
    scheduleIdle();
  });

  let lastWink = 0;
  $effect(() => {
    if (winkTick === lastWink) return;
    lastWink = winkTick;
    if (mood !== "attention" && !holding) play("wink", () => restingPose);
  });

  onMount(() => {
    for (const id of [...Object.keys(manifest.frames), ...Object.keys(overlays)]) {
      const img = new Image();
      img.src = frameUrl(id);
    }
    return () => {
      stop();
      if (idle) clearTimeout(idle);
    };
  });
</script>

<span class="robot" style:width="{size}px" style:height="{(size * BASE.height) / BASE.width}px">
  <img src={frameUrl(frame)} width={size} alt="" draggable="false" />
  {#if overlay}
    {@const o = overlays[overlay]}
    <img
      class="bubble"
      src={frameUrl(overlay)}
      width={(o.width * size) / BASE.width}
      style:left="{(o.left * size) / BASE.width}px"
      alt=""
      draggable="false"
    />
  {/if}
</span>

<style>
  .robot {
    position: relative;
    display: block;
    pointer-events: none;
  }
  img {
    position: absolute;
    left: 0;
    bottom: 0;
    display: block;
    height: auto;
    -webkit-user-drag: none;
  }
</style>
