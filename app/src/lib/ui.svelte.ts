// UI state shared across the panel views: short notices,
// dialogs and conversation actions.
import { api } from "./api";
import type { AccessResult, ConversationCard } from "./types";

export type Toast = { id: number; message: string; action?: { label: string; run: () => void } };

export type Dialog =
  | { kind: "rename"; card: ConversationCard }
  | { kind: "move"; card: ConversationCard }
  | { kind: "unreachable"; title: string; message: string; command: string | null }
  | { kind: "needsFolder"; card: ConversationCard; message: string }
  | { kind: "project"; id: number; name: string; description: string | null }
  | { kind: "merge"; id: number; name: string };

export const ui = $state<{ toast: Toast | null; dialog: Dialog | null; busy: Record<number, boolean> }>({
  toast: null,
  dialog: null,
  busy: {},
});

let toastTimer: ReturnType<typeof setTimeout> | null = null;

export function toast(message: string, action?: Toast["action"], ms = 6000) {
  ui.toast = { id: Date.now(), message, action };
  holdToast(ms);
}

/** Stops the auto-close timer; with `ms`, restarts it from now. */
export function holdToast(ms?: number) {
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = ms === undefined ? null : setTimeout(dismissToast, ms);
}

export function dismissToast() {
  holdToast();
  ui.toast = null;
}

export function failure(e: unknown) {
  toast(typeof e === "string" ? e : "Something went wrong. Try again.");
}

function handle(r: AccessResult, card: ConversationCard) {
  switch (r.outcome) {
    case "focused":
      break;
    case "appActivated":
      toast(r.note);
      break;
    case "resumed":
      toast(`Conversation resumed in ${r.app}.`);
      break;
    case "unreachable":
      ui.dialog = { kind: "unreachable", title: "Can't reach this terminal", message: r.message, command: r.command };
      break;
    case "needsFolder":
      ui.dialog = { kind: "needsFolder", card, message: r.message };
      break;
  }
}

/** Main card action: Open conversation or Resume. */
export async function primaryAction(card: ConversationCard) {
  if (ui.busy[card.id]) return;
  ui.busy[card.id] = true;
  try {
    handle(card.canResume ? await api.resume(card.id) : await api.open(card.id), card);
  } catch (e) {
    failure(e);
  } finally {
    delete ui.busy[card.id];
  }
}

export async function resumeInFolder(card: ConversationCard) {
  ui.dialog = null;
  try {
    const r = await api.chooseFolderAndResume(card.id);
    if (r) handle(r, card);
  } catch (e) {
    failure(e);
  }
}

export async function removeFromView(card: ConversationCard) {
  try {
    await api.hide(card.id);
    toast("Removed from view. If the conversation picks up again, it will reappear here.", {
      label: "Undo",
      run: () => api.unhide(card.id),
    });
  } catch (e) {
    failure(e);
  }
}

export async function restore(card: ConversationCard) {
  try {
    await api.unhide(card.id);
    toast("Conversation restored.");
  } catch (e) {
    failure(e);
  }
}
