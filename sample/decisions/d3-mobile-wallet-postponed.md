# Mobile wallet integration postponed to Q3

The mobile wallet integration is postponed to Q3.

**Reasoning:** The wallet vendor's API contracts are not stable yet — the
authentication flow changed twice during integration spikes, and the sandbox
returned inconsistent transaction statuses. Aisha and Elena agreed that
shipping against a moving contract would create rework and user-facing bugs,
so the decision was made to defer rather than block the rest of the roadmap.

**Impact:**
- The payments service keeps the existing card flows for Q2.
- The design team (Elena) shelves the wallet UX work until contracts freeze.
- Partner engagement is on hold; Aisha owns the re-engagement when the vendor
  signals stability.

**Status:** Confirmed. Source: sprint 12 planning notes. Owner: Aisha.

**Revisit trigger:** vendor freezes auth API or provides a versioned contract.
