export type Route =
  | { name: "overview" }
  | { name: "project"; id: number | null }
  | { name: "removed" }
  | { name: "settings" }
  | { name: "activity" }
  | { name: "doctor" }
  | { name: "onboarding" };
