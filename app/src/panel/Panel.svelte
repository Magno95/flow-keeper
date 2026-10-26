<script lang="ts">
  import { onMount } from "svelte";
  import { api, onChanged, onNavigate, onPanelClosed, onPanelOpened } from "../lib/api";
  import type { SetupState } from "../lib/types";
  import Dialogs from "./Dialogs.svelte";
  import DoctorView from "./DoctorView.svelte";
  import Onboarding from "./Onboarding.svelte";
  import Overview from "./Overview.svelte";
  import ProjectView from "./ProjectView.svelte";
  import RemovedView from "./RemovedView.svelte";
  import SettingsView from "./SettingsView.svelte";
  import Toast from "./Toast.svelte";
  import type { Route } from "./routes";
  import { ui } from "../lib/ui.svelte";

  let route = $state<Route>({ name: "overview" });
  let refresh = $state(0);
  let opened = $state(0);
  let closed = $state(0);
  let setup = $state<SetupState | null>(null);

  const go = (r: Route) => (route = r);

  async function loadSetup() {
    setup = await api.setupState().catch(() => null);
    if (setup && !setup.onboardingDone && route.name === "overview") route = { name: "onboarding" };
  }

  onMount(() => {
    loadSetup();
    const subs = [
      onChanged(() => refresh++),
      onPanelOpened(() => {
        opened++;
        loadSetup();
      }),
      // On reopening, always start from a clean overview (no search, no scroll);
      // the guided setup stays at the step it was on.
      onPanelClosed(() => {
        if (route.name === "onboarding") return;
        route = { name: "overview" };
        closed++;
      }),
      onNavigate((to) => {
        if (to === "doctor") route = { name: "doctor" };
        else if (to === "updates") route = { name: "settings" };
      }),
    ];
    return () => subs.forEach((p) => p.then((f) => f()));
  });

  function keys(e: KeyboardEvent) {
    if (e.key !== "Escape" || ui.dialog) return;
    if (route.name === "overview" || route.name === "onboarding") api.hidePanel();
    else if (route.name === "doctor") route = { name: "settings" };
    else route = { name: "overview" };
  }
</script>

<svelte:window onkeydown={keys} />

<main class="shell">
  {#if route.name === "overview"}
    {#key closed}
      <Overview {refresh} {opened} {setup} {go} />
    {/key}
  {:else if route.name === "project"}
    {#key route.id}
      <ProjectView id={route.id} {refresh} {opened} {go} />
    {/key}
  {:else if route.name === "removed"}
    <RemovedView {refresh} {go} />
  {:else if route.name === "settings"}
    <SettingsView {go} />
  {:else if route.name === "doctor"}
    <DoctorView {go} />
  {:else if route.name === "onboarding"}
    <Onboarding {refresh} {go} />
  {/if}
  <Dialogs />
  <Toast />
</main>

<style>
  .shell {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-radius: 16px;
    overflow: hidden;
    border: 1px solid var(--line);
  }
</style>
