// GraphMini — signature memory-graph motif (R16.8, SVG rebuilt R18.17, rebuilt
// as HTML in R18.6). Entity nodes chained by labelled edges plus a pruned
// fan-out branch. It is real HTML styled by the `.graph*` rules in style.css
// instead of a viewBox-scaled SVG: the SVG rendered at 0.95x on desktop and
// ~0.68x on phones, putting its 10px labels at 9.5px and 6.8px. Real type keeps
// every label >= 11.5px at any width, and the chain stacks vertically on
// phones instead of shrinking. Stone & Sage tokens only, no new hex.
// Renders into a container by id; two instances ship on the same page.
export function renderGraphMini(containerId: string): void {
  const el = document.getElementById(containerId);
  if (!el) return;
  el.innerHTML = `
  <div class="graph">
    <div class="graph-row">
      <div class="gnode"><b>Sarah</b><small>payments lead</small></div>
      <div class="gedge"><span>owns</span></div>
      <div class="gnode"><b>billing migration</b><small>decision · 87%</small></div>
      <div class="gedge dashed"><span>blocked by</span></div>
      <div class="gnode warn"><b>retention gap</b><small>risk · closed</small></div>
    </div>
    <div class="graph-prune"><span class="gnode pruned"><b>pruned · cap</b></span></div>
    <p class="graph-foot">RRF k=60 + age decay + diversity cap → cited answer · every hop audited</p>
  </div>`;
}
