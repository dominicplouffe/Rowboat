<script lang="ts">
  // The Rowboat mark (assets/brand/logo.svg), inlined so its outline can follow
  // the theme: navy on light backgrounds, light on dark ones.
  let { width = 96 }: { width?: number } = $props();
  const HULL = 'M172 470Q500 492 884 424C852 560 770 668 640 668H322C240 668 190 596 172 470Z';
</script>

<svg {width} height={(width * 568) / 776} viewBox="140 286 776 568" aria-hidden="true">
  <clipPath id="logo-hull"><path d={HULL} /></clipPath>
  <g clip-path="url(#logo-hull)">
    <rect x="150" y="410" width="760" height="280" fill="#E8A33D" />
    <rect x="150" y="522" width="760" height="42" fill="#FFF4E0" />
    <rect x="150" y="606" width="760" height="42" fill="#FFF4E0" />
  </g>
  <g fill="none" stroke="var(--logo-ink)" stroke-linecap="round" stroke-linejoin="round">
    <path d={HULL} stroke-width="30" />
    <path d="M478 318L338 748M664 318L524 748" stroke-width="30" />
    <path d="M374 638L338 748M560 638L524 748" stroke-width="72" />
    <path d="M374 638L338 748M560 638L524 748" stroke="#E8A33D" stroke-width="34" />
    <path
      d="M176 768C232 720 288 720 344 768S456 816 512 768S624 720 680 768S792 816 848 768"
      stroke="#2D9CDB"
      stroke-width="44"
    />
  </g>
</svg>
