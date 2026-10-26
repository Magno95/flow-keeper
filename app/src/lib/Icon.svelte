<script lang="ts">
  let { name, size = 18 }: { name: string; size?: number } = $props();
  const paths: Record<string, string> = {
    back: "M15 18l-6-6 6-6",
    chevron: "M9 6l6 6-6 6",
    search: "M11 4a7 7 0 1 1 0 14 7 7 0 0 1 0-14zm9 16l-4.3-4.3",
    settings:
      "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm7.4-3a7.4 7.4 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7.5 7.5 0 0 0-2-1.2L14.5 3h-5l-.4 2.6a7.5 7.5 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7.4 7.4 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7.5 7.5 0 0 0 2 1.2l.4 2.6h5l.4-2.6a7.5 7.5 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2z",
    more: "M5 12h.01M12 12h.01M19 12h.01",
    close: "M6 6l12 12M18 6L6 18",
    check: "M5 12.5l4.5 4.5L19 7.5",
    warn: "M12 4l9 16H3l9-16zm0 6v4m0 3h.01",
    error: "M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zm0 5v5m0 3h.01",
    info: "M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zm0 8v5m0-8h.01",
    restore: "M4 12a8 8 0 1 0 2.3-5.7M4 4v4h4",
    terminal: "M4 5h16v14H4zM8 10l3 2-3 2m5 1h3",
    copy: "M9 9h10v10H9zM5 15V5h10",
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={name === "more" ? 3 : 1.8}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name] ?? ""} />
</svg>
