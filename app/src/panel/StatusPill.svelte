<script lang="ts">
  import type { DisplayState } from "../lib/types";
  let { state, label }: { state: DisplayState; label: string } = $props();
</script>

<span class="pill {state}"><span class="dot" aria-hidden="true"></span>{label}</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px 3px 8px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    background: var(--neutral-bg);
    color: var(--neutral);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: currentColor;
  }
  .working {
    background: var(--working-bg);
    color: var(--working);
  }
  .waiting {
    background: var(--waiting-bg);
    color: var(--waiting);
  }
  .finished {
    background: var(--finished-bg);
    color: var(--finished);
  }
  .closed .dot {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px currentColor;
  }
  .unknown .dot {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px currentColor;
    border-radius: 2px;
  }
</style>
