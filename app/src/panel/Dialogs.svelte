<script lang="ts">
  import { api } from "../lib/api";
  import { failure, resumeInFolder, toast, ui } from "../lib/ui.svelte";
  import type { ProjectOption } from "../lib/types";

  let text = $state("");
  let desc = $state("");
  let target = $state<string>("");
  let newName = $state("");
  let associate = $state(false);
  let options = $state<ProjectOption[]>([]);
  let showCommand = $state(false);
  let dialogEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    const d = ui.dialog;
    showCommand = false;
    if (!d) return;
    if (d.kind === "rename") text = d.card.titleIsProvisional ? "" : d.card.title;
    if (d.kind === "project") {
      text = d.name;
      desc = d.description ?? "";
    }
    if (d.kind === "move" || d.kind === "merge") {
      target = "";
      newName = "";
      associate = d.kind === "move" && d.card.projectId === null;
      api.projects().then((p) => (options = p), failure);
    }
    queueMicrotask(() => dialogEl?.querySelector<HTMLElement>("input, select, button.btn")?.focus());
  });

  const close = () => (ui.dialog = null);

  async function save() {
    const d = ui.dialog;
    if (!d) return;
    try {
      if (d.kind === "rename") {
        await api.rename(d.card.id, text.trim() || null);
        toast(text.trim() ? "Title updated." : "Back to the generated title.");
      } else if (d.kind === "move") {
        const isNew = target === "new";
        await api.move({
          id: d.card.id,
          projectId: isNew || target === "" ? null : Number(target),
          newProjectName: isNew ? newName.trim() : null,
          associateFolder: associate,
        });
        toast("Conversation moved.");
      } else if (d.kind === "project") {
        await api.updateProject(d.id, text.trim() || null, desc.trim() || null, true);
      } else if (d.kind === "merge") {
        if (!target) return;
        await api.mergeProjects(d.id, Number(target));
        toast(`“${d.name}” merged.`);
      }
      close();
    } catch (e) {
      failure(e);
    }
  }

  function keys(e: KeyboardEvent) {
    if (ui.dialog && e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  }

  async function copy(cmd: string) {
    await navigator.clipboard.writeText(cmd);
    toast("Command copied.");
  }
</script>

<svelte:window onkeydown={keys} />

{#if ui.dialog}
  {@const d = ui.dialog}
  <div class="backdrop" role="presentation" onclick={close}></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dlg-title" bind:this={dialogEl}>
    {#if d.kind === "rename"}
      <h2 id="dlg-title">Edit title</h2>
      <p class="hint">The title you choose won't be overwritten by summaries. Leave it empty to go back to the generated one.</p>
      <input class="field" maxlength="80" bind:value={text} placeholder={d.card.title} onkeydown={(e) => e.key === "Enter" && save()} />
      <div class="row"><button class="btn secondary" onclick={close}>Cancel</button><button class="btn" onclick={save}>Save</button></div>
    {:else if d.kind === "move"}
      <h2 id="dlg-title">Move to another project</h2>
      <p class="hint">Moves only this conversation: the others stay where they are.</p>
      <select class="field" bind:value={target}>
        <option value="" disabled>Choose a project</option>
        {#each options.filter((o) => o.id !== d.card.projectId) as o}
          <option value={String(o.id)}>{o.name}</option>
        {/each}
        <option value="new">New project…</option>
      </select>
      {#if target === "new"}
        <input class="field" maxlength="60" bind:value={newName} placeholder="Project name" />
      {/if}
      <label class="check">
        <input type="checkbox" bind:checked={associate} />
        <span>Also use this project for future conversations in the same folder</span>
      </label>
      <div class="row">
        <button class="btn secondary" onclick={close}>Cancel</button>
        <button class="btn" disabled={!target || (target === "new" && !newName.trim())} onclick={save}>Move</button>
      </div>
    {:else if d.kind === "project"}
      <h2 id="dlg-title">Project</h2>
      <input class="field" maxlength="60" bind:value={text} placeholder="Name" />
      <input class="field" maxlength="160" bind:value={desc} placeholder="Short description (optional)" />
      <div class="row"><button class="btn secondary" onclick={close}>Cancel</button><button class="btn" onclick={save}>Save</button></div>
    {:else if d.kind === "merge"}
      <h2 id="dlg-title">Merge “{d.name}”</h2>
      <p class="hint">Its conversations and linked folders move to the chosen project. Each conversation keeps its own summary.</p>
      <select class="field" bind:value={target}>
        <option value="" disabled>Choose the destination project</option>
        {#each options.filter((o) => o.id !== d.id) as o}
          <option value={String(o.id)}>{o.name}</option>
        {/each}
      </select>
      <div class="row"><button class="btn secondary" onclick={close}>Cancel</button><button class="btn" disabled={!target} onclick={save}>Merge</button></div>
    {:else if d.kind === "unreachable"}
      <h2 id="dlg-title">{d.title}</h2>
      <p class="hint">{d.message}</p>
      {#if d.command}
        <button class="disclose" onclick={() => (showCommand = !showCommand)} aria-expanded={showCommand}>
          {showCommand ? "Hide" : "Show"} the command to resume it manually
        </button>
        {#if showCommand}
          <div class="cmd"><code>{d.command}</code><button class="btn small secondary" onclick={() => copy(d.command!)}>Copy</button></div>
        {/if}
      {/if}
      <div class="row"><button class="btn" onclick={close}>Got it</button></div>
    {:else if d.kind === "needsFolder"}
      <h2 id="dlg-title">Folder not found</h2>
      <p class="hint">{d.message}</p>
      <div class="row">
        <button class="btn secondary" onclick={close}>Cancel</button>
        <button class="btn" onclick={() => resumeInFolder(d.card)}>Choose folder</button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .backdrop {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.22);
    z-index: 40;
  }
  .dialog {
    position: absolute;
    left: 16px;
    right: 16px;
    top: 50%;
    transform: translateY(-50%);
    z-index: 41;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-pop);
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 17px;
  }
  .hint {
    margin: 0;
    color: var(--text-2);
    font-size: 13px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    font-size: 13px;
    color: var(--text-2);
  }
  .disclose {
    align-self: flex-start;
    border: 0;
    background: transparent;
    padding: 0;
    color: var(--text-2);
    font-size: 12.5px;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .cmd {
    display: flex;
    gap: 8px;
    align-items: center;
    background: var(--surface-muted);
    border-radius: var(--radius-sm);
    padding: 8px 10px;
  }
  code {
    flex: 1;
    font-size: 11.5px;
    word-break: break-all;
    -webkit-user-select: text;
    user-select: text;
  }
</style>
