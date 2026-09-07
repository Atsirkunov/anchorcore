// GraphMini — signature memory-graph motif (R16.8): entity nodes with citation
// ticks + a pruned fan-out branch. Stone & Sage, no new hex (CSS vars only).
// Renders into a container by id; mount a second instance with another id.
export function renderGraphMini(containerId: string): void {
  const el = document.getElementById(containerId);
  if (!el) return;
  // Use CSS vars so palette stays single-source (C3 Stone & Sage)
  el.innerHTML = `
  <svg viewBox="0 0 640 190" width="100%" height="190" role="img" aria-labelledby="gTitle gDesc" style="display:block">
    <title id="gTitle">Graph walk — 3 cited nodes, pruned fan-out</title>
    <desc id="gDesc">Supersedes (1.0) → depends_on (0.9) → owns (0.8), pruned branch capped, stitched citations</desc>
    <rect width="640" height="190" rx="12" fill="var(--paper)" stroke="var(--border)"/>
    <!-- edges -->
    <line x1="140" y1="68" x2="250" y2="68" stroke="var(--sage)" stroke-width="1.6" opacity="0.9"/>
    <line x1="390" y1="68" x2="500" y2="68" stroke="var(--stone)" stroke-width="1.4" stroke-dasharray="4 4" opacity="0.85"/>
    <line x1="195" y1="72" x2="195" y2="108" stroke="var(--faint)" stroke-width="1.4" stroke-dasharray="3 3"/>
    <!-- pruned branch -->
    <rect x="145" y="108" width="100" height="30" rx="8" fill="var(--surface)" stroke="var(--border)" stroke-dasharray="4 3" opacity="0.8"/>
    <text x="195" y="127" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="8" fill="var(--faint)">pruned · cap</text>
    <!-- nodes (citation ticks = stitched sources) -->
    <g>
      <rect x="40" y="40" width="100" height="56" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <rect x="48" y="48" width="3" height="40" rx="1.5" fill="var(--sage)"/>
      <circle cx="90" cy="56" r="6" fill="var(--sage)" stroke="white" stroke-width="2"/>
      <text x="90" y="76" text-anchor="middle" font-family="Georgia,serif" font-size="11" font-weight="700" fill="var(--ink)">DVCA</text>
      <text x="90" y="88" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--dim)">decision 82%</text>
    </g>
    <g>
      <text x="195" y="58" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--sage)" font-weight="700">supersedes 1.0</text>
    </g>
    <g>
      <rect x="250" y="40" width="140" height="56" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <rect x="258" y="48" width="3" height="40" rx="1.5" fill="var(--stone)"/>
      <circle cx="320" cy="56" r="6" fill="var(--stone)" stroke="white" stroke-width="2"/>
      <text x="320" y="76" text-anchor="middle" font-family="Georgia,serif" font-size="11" font-weight="700" fill="var(--ink)">billing migration</text>
      <text x="320" y="88" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--dim)">owns · Sarah</text>
    </g>
    <g>
      <text x="445" y="58" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--stone)" font-weight="700">depends_on 0.9</text>
    </g>
    <g>
      <rect x="500" y="40" width="100" height="56" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <rect x="508" y="48" width="3" height="40" rx="1.5" fill="var(--sage)"/>
      <circle cx="550" cy="56" r="6" fill="var(--sage)" stroke="white" stroke-width="2"/>
      <text x="550" y="76" text-anchor="middle" font-family="Georgia,serif" font-size="11" font-weight="700" fill="var(--ink)">reconciliation</text>
      <text x="550" y="88" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="9" fill="var(--dim)">note</text>
    </g>
    <!-- footnote -->
    <text x="20" y="166" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--dim)">RRF k=60 + age decay + diversity cap → cited answer · every hop audited</text>
  </svg>`;
}
