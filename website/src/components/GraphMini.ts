// GraphMini — 3 nodes, Stone & Sage, no new hex (uses CSS vars --sage --stone --paper --border)
// Renders into a container by id. Keeps CCN <=15, no backend.
export function renderGraphMini(containerId: string): void {
  const el = document.getElementById(containerId);
  if (!el) return;
  // Use CSS vars so palette stays single-source (C3 Stone & Sage)
  el.innerHTML = `
  <svg viewBox="0 0 640 160" width="100%" height="160" role="img" aria-labelledby="gTitle gDesc" style="display:block">
    <title id="gTitle">Graph walk — 3 nodes, 1–2 hops</title>
    <desc id="gDesc">Supersedes (1.0) → depends_on (0.9) → owns (0.8), fan-out capped, stitched citations</desc>
    <rect width="640" height="160" rx="12" fill="var(--paper)" stroke="var(--border)"/>
    <!-- edges -->
    <line x1="140" y1="80" x2="250" y2="80" stroke="var(--sage)" stroke-width="1.6" opacity="0.9"/>
    <line x1="390" y1="80" x2="500" y2="80" stroke="var(--stone)" stroke-width="1.4" stroke-dasharray="4 4" opacity="0.85"/>
    <!-- nodes -->
    <g>
      <rect x="40" y="52" width="100" height="56" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <circle cx="90" cy="68" r="6" fill="var(--sage)" stroke="white" stroke-width="2"/>
      <text x="90" y="88" text-anchor="middle" font-family="Georgia,serif" font-size="11" font-weight="700" fill="var(--ink)">DVCA</text>
      <text x="90" y="100" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--dim)">decision 82%</text>
    </g>
    <g>
      <text x="195" y="84" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--sage)" font-weight="700">supersedes 1.0</text>
      <text x="195" y="96" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="8" fill="var(--faint)">1 hop</text>
    </g>
    <g>
      <rect x="250" y="52" width="140" height="56" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <circle cx="320" cy="68" r="6" fill="var(--stone)" stroke="white" stroke-width="2"/>
      <text x="320" y="88" text-anchor="middle" font-family="Georgia,serif" font-size="11" font-weight="700" fill="var(--ink)">billing migration</text>
      <text x="320" y="100" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--dim)">owns · Sarah</text>
    </g>
    <g>
      <text x="445" y="84" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--stone)" font-weight="700">depends_on 0.9</text>
      <text x="445" y="96" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="8" fill="var(--faint)">2 hops</text>
    </g>
    <g>
      <rect x="500" y="52" width="100" height="56" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <circle cx="550" cy="68" r="6" fill="var(--sage)" stroke="white" stroke-width="2"/>
      <text x="550" y="88" text-anchor="middle" font-family="Georgia,serif" font-size="11" font-weight="700" fill="var(--ink)">reconciliation</text>
      <text x="550" y="100" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--dim)">note</text>
    </g>
    <!-- footnote -->
    <text x="20" y="136" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--dim)">RRF k=60 + age decay + diversity cap → cited answer · </text>
    <text x="20" y="150" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--faint)">who owns X? via who_knows · every hop audited</text>
  </svg>`;
}
