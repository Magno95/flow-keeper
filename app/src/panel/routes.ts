export type Route =
  | { name: "overview" }
  | { name: "project"; id: number | null }
  | { name: "removed" }
  | { name: "settings" }
  | { name: "doctor" }
  | { name: "onboarding" };
