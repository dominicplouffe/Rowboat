<script lang="ts" module>
  // A handful of 16px stroke icons, inlined so nothing is fetched or bundled
  // beyond what the UI actually uses.
  const PATHS = {
    plus: 'M8 3v10M3 8h10',
    close: 'M4 4l8 8M12 4l-8 8',
    settings:
      'M8 10.2a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4ZM8 1.8v1.6M8 12.6v1.6M2.6 4.9l1.4.8M12 10.3l1.4.8M2.6 11.1l1.4-.8M12 5.7l1.4-.8',
    play: 'M5 3.5v9l7-4.5-7-4.5Z',
    sidebar: 'M2.5 3.5h11v9h-11zM6 3.5v9',
    search: 'M7 11.5a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9ZM10.4 10.4l3.1 3.1',
    database:
      'M3 4.2C3 3.1 5.2 2.2 8 2.2s5 .9 5 2-2.2 2-5 2-5-.9-5-2ZM3 4.2v7.6c0 1.1 2.2 2 5 2s5-.9 5-2V4.2M3 8c0 1.1 2.2 2 5 2s5-.9 5-2',
    chevron: 'M4 6l4 4 4-4',
    check: 'M3.5 8.5l3 3 6-7',
    reset: 'M3 8a5 5 0 1 0 1.6-3.7M3 2.8v2.4h2.4',
  } as const;
  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.4"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={PATHS[name]} />
</svg>
