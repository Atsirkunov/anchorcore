// GraphMini — signature memory-graph motif (R16.8, rebuilt R18.17): entity nodes
// with citation ticks + a pruned fan-out branch. Stone & Sage, no new hex (CSS
// vars only). Labels sit right of the citation tick (same language as the answer
// cards in the hero) so text can never collide with the bar; card widths are sized
// to their longest label and edge gaps to their longest edge label.
// Renders into a container by id; mount a second instance with another id.
export function renderGraphMini(containerId: string): void {
  const el = document.getElementById(containerId);
  if (!el) return;
  // unique ids per instance (two graphs ship on the same page)
  const titleId = `${containerId}-title`;
  const descId = `${containerId}-desc`;
  el.innerHTML = `
  <svg viewBox="0 0 520 152" width="100%" role="img" aria-labelledby="${titleId} ${descId}" style="display:block;height:auto">
    <title id="${titleId}">Graph walk — Sarah owns the billing migration, blocked by the retention gap; pruned fan-out</title>
    <desc id="${descId}">Owner node owns the decision node, which is blocked by a risk node; one branch is pruned, every hop is cited</desc>
    <rect width="520" height="152" rx="12" fill="var(--paper)" stroke="var(--border)"/>
    <!-- edges -->
    <line x1="124" y1="53" x2="164" y2="53" stroke="var(--sage)" stroke-width="1.6" opacity="0.9"/>
    <line x1="316" y1="53" x2="378" y2="53" stroke="var(--stone)" stroke-width="1.4" stroke-dasharray="4 4" opacity="0.85"/>
    <text x="144" y="45" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--sage)" font-weight="700">owns</text>
    <text x="347" y="45" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--stone)" font-weight="700">blocked by</text>
    <!-- pruned branch -->
    <line x1="144" y1="57" x2="144" y2="98" stroke="var(--faint)" stroke-width="1.4" stroke-dasharray="3 3"/>
    <rect x="94" y="98" width="100" height="28" rx="8" fill="var(--surface)" stroke="var(--border)" stroke-dasharray="4 3" opacity="0.85"/>
    <text x="144" y="116" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--faint)">pruned · cap</text>
    <!-- node: owner -->
    <g>
      <rect x="14" y="24" width="110" height="58" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <rect x="23" y="32" width="3" height="42" rx="1.5" fill="var(--sage)"/>
      <text x="32" y="50" font-family="Georgia,serif" font-size="12.5" font-weight="700" fill="var(--ink)">Sarah</text>
      <text x="32" y="68" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--dim)">payments lead</text>
    </g>
    <!-- node: decision -->
    <g>
      <rect x="164" y="24" width="152" height="58" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <rect x="173" y="32" width="3" height="42" rx="1.5" fill="var(--sage)"/>
      <text x="182" y="50" font-family="Georgia,serif" font-size="12.5" font-weight="700" fill="var(--ink)">billing migration</text>
      <text x="182" y="68" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--dim)">decision · 87%</text>
    </g>
    <!-- node: risk -->
    <g>
      <rect x="378" y="24" width="128" height="58" rx="10" fill="var(--surface)" stroke="var(--border)"/>
      <rect x="387" y="32" width="3" height="42" rx="1.5" fill="var(--stone)"/>
      <text x="396" y="50" font-family="Georgia,serif" font-size="12.5" font-weight="700" fill="var(--ink)">retention gap</text>
      <text x="396" y="68" font-family="ui-monospace,Menlo,monospace" font-size="10" fill="var(--dim)">risk · closed</text>
    </g>
    <!-- footnote -->
    <text x="14" y="142" font-family="ui-monospace,Menlo,monospace" font-size="10.5" fill="var(--dim)">RRF k=60 + age decay + diversity cap → cited answer · every hop audited</text>
  </svg>`;
}
