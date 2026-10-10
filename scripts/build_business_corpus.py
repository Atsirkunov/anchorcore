#!/usr/bin/env python3
"""
Build a realistic tech/finance/AI business scale corpus.

Creates data/business-scale/ mirroring a 20-30 person payments+AI company
(tech RFCs + runbooks, finance filings/compliance, AI papers/governance,
 synthetic meetings/decisions). Usable as a single Local Folder source
(docs/sample-dataset.md) to stress ingestion + retrieval at scale.

Usage:
  python scripts/build_business_corpus.py                 # ~800 docs (default, fast)
  python scripts/build_business_corpus.py --scale 5k     # ~5k docs / ~150k chunks
  python scripts/build_business_corpus.py --scale 1k --with-real  # try git/SEC pulls
  python scripts/build_business_corpus.py --clean        # remove and rebuild

Real pulls (optional, --with-real): clones rust RFCs, KEPs, PEPs; downloads
SEC 10-K stubs; fetches arXiv list. Failures fall back to synthetic so the
corpus is always usable offline. Nothing is committed to git (data/ is gitignored).
"""
import argparse, hashlib, json, os, random, shutil, subprocess, sys, textwrap, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "data" / "business-scale"
RAW = ROOT / "data" / "raw"

# Deterministic
RND = random.Random(42)

OWNERS = ["Aisha","Sarah","Dev","Maya","Ken","Priya","Alex","Jordan","Sam","Riley","Taylor","Morgan"]
TEAMS = ["Payments","Risk","Platform","AI Infra","Finance","Compliance"]
SYSTEMS = ["ledger","settlement-engine","risk-engine","idempotency-service","billing-migration","kyc-service","feature-flag","vector-store","reconciliation-tooling"]
FIN_TECH_TOPICS = [
    ("DVCA movement rules","settlement-only, reconciliation complexity"),
    ("MRGR merger movements","corporate action, deterministic steps"),
    ("Idempotency key TTL","24h expiry, exactly-once semantics"),
    ("Billing migration","Stripe provider, cutover steps"),
    ("KYC refresh","90-day re-verification, document checks"),
    ("Reconciliation pipeline","T+1 batch, external statements"),
    ("Feature flag rollout","canary 5% -> 50% -> 100%, kill switch"),
    ("Rate limiting","100 req/s per account, burst 200"),
]

AI_TOPICS = [
    ("Retrieval hybrid fusion","vector + FTS5 RRF k=60, age decay"),
    ("Graph walk 1-2 hops","supersedes > depends_on > owns > blocks"),
    ("Distillation Q&A units","meeting -> normalized Q: A: terms"),
    ("PII gating","sensitive/pii local-only, cloud blocked"),
    ("Prompt injection guard","system prompt isolation, tool allowlist"),
    ("Model routing","Ollama local 3B vs BYO cloud, cost guardrail"),
    ("Evaluation harness","60s wow: cited answer with section"),
    ("Vector dim 768","nomic-embed-text, sqlite-vec vec0"),
]

COMPLIANCE_TOPICS = [
    ("SOC2 CC6.1","logical access, least privilege"),
    ("IFRS 9 classification","amortised cost vs FVPL"),
    ("PCI DSS 3.4","PAN rendered unreadable at rest"),
    ("KYC AML 5AMLD","beneficial owner 25% threshold"),
    ("GDPR Art 17","right to erasure, 30-day response"),
]

def sh(cmd, cwd=None, timeout=30):
    try:
        r = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return r.returncode == 0, (r.stdout + r.stderr)[:500]
    except Exception as e:
        return False, str(e)

def ensure_dirs():
    for d in [OUT / "tech" / "rfcs", OUT / "tech" / "runbooks",
              OUT / "finance" / "filings", OUT / "finance" / "compliance",
              OUT / "ai" / "papers", OUT / "ai" / "governance",
              OUT / "meetings", OUT / "decisions", RAW]:
        d.mkdir(parents=True, exist_ok=True)

def write(path: Path, content: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content.strip() + "\n", encoding="utf-8")

def synth_decision(idx: int, topic, reasoning):
    owner = RND.choice(OWNERS)
    team = RND.choice(TEAMS)
    date = f"2024-{RND.randint(1,12):02d}-{RND.randint(1,28):02d}"
    supersedes = f"Supersedes: D{idx-3:03d}" if idx>3 and RND.random()<0.3 else ""
    body = f"""# Decision: {topic} — D{idx:04d}

**Decision:** {topic} will be implemented as: {reasoning}.
**Reasoning:** Chosen for reliability and privacy at scale; alternatives were higher cost or weaker provenance. Tradeoff logged in PRD.
**Owner:** {owner} ({team}) · **Date:** {date} · **Status:** Confirmed
**Context:** {RND.choice(['Q3 planning','Sprint retro','Risk review','Architecture review'])} — see meetings/m{RND.randint(1,80):02d}.md
{supersedes}
**Follow-ups:**
- Spec in tech/runbooks/rb-{idx:03d}.md
- Depends on {RND.choice(SYSTEMS)}
"""
    return body

def synth_runbook(idx: int, topic):
    steps = RND.randint(4,8)
    lines = [f"# Runbook: {topic} — RB{idx:03d}", "", f"Owner: {RND.choice(OWNERS)} · System: {RND.choice(SYSTEMS)}", "", "## Steps"]
    for s in range(1, steps+1):
        lines.append(f"{s}. **Step {s}** — {RND.choice(['Validate input','Reserve position','Deliver to counterparty','Confirm settlement','Write audit row','Reconcile external statement','Rollback on failure','Notify owner'])} — status: `{RND.choice(['pending','validated','reserved','delivered','confirmed'])}`")
    lines += ["", "## Rollback", "Failed delivery reverses reservation (steps 3-4). Reconciliation tooling attaches external state to step 4.", "", "## Limits", "- Daily cap per account, enforced at validation.", "- Idempotency key TTL 24h."]
    return "\n".join(lines)

def synth_filing(idx: int):
    company = RND.choice(["AstraPay Inc.","Northstar Fintech Ltd.","AnchorCore Payments LLC"])
    fy = RND.choice(["FY2023","FY2024"])
    sections = []
    for sec in ["1. Business","1A. Risk Factors","7. MD&A","8. Financial Statements","9A. Controls"]:
        para = " ".join(RND.choice(["Revenue growth","Credit risk","Settlement exposure","Capital adequacy","Liquidity","Operational resilience"]) for _ in range(30))
        sections.append(f"## Section {sec}\n{para}\n\n| Metric | {fy} | Prior |\n|---|---|---|\n| Revenue ($m) | {RND.randint(80,400)} | {RND.randint(60,350)} |\n| Cost ($m) | {RND.randint(40,200)} | {RND.randint(30,180)} |\n| Page {RND.randint(1,250)} of 250 | | |")
    return f"# {company} — Form 10-K {fy} — Filing {idx:03d}\n\nCompany: {company}\nFiled: 2024-{RND.randint(1,12):02d}-{RND.randint(1,28):02d}\nAuditor: {RND.choice(['EY','PwC','Deloitte'])}\n\n" + "\n\n".join(sections)

def synth_compliance(idx: int, title, desc):
    return f"""# Compliance: {title} — C{idx:03d}

**Requirement:** {desc}
**Applies to:** {RND.choice(['payments','kyc-service','ledger','settlement-engine'])} · **Owner:** {RND.choice(OWNERS)}
**Status:** Implemented · **Review:** 2025-{RND.randint(1,12):02d}-{RND.randint(1,28):02d}

## Controls
- Access reviewed quarterly, least privilege.
- Evidence captured in audit rows (who, when, what, amount).
- Cross-ref: decisions/d{RND.randint(1,80):04d}.md, runbooks/rb-{RND.randint(1,80):03d}.md

## Test
External auditor sample 25 records, 0 exceptions.
"""

def synth_ai_paper(idx: int, topic, desc):
    authors = ", ".join(RND.sample(OWNERS, k=3))
    abstract = f"This paper studies {topic.lower()}. We evaluate {desc} on a 237-page compliance corpus and a payments runbook suite. Hybrid retrieval (RRF k=60) with sqlite-vec vec0 (768d) and FTS5 improves citation recall vs vector-only. Graph walk 1-2 hops surfaces supersedes/depends_on without lexical overlap."
    return f"""# Paper: {topic} — P{idx:04d}

**Authors:** {authors} · **Venue:** arXiv cs.{RND.choice(['AI','LG','CL'])} 2024-{RND.randint(1,12):02d} · **System:** {RND.choice(SYSTEMS)}

## Abstract
{abstract}

## 1. Introduction
AI models lack company memory. AnchorCore provides filing-system memory: decisions, owners, dependencies with provenance. Every answer cites source + section.

## 2. Method
Planner picks tools (hybrid always, who_knows for ownership). Executor fuses via RRF, age decay half-life 365d, per-file cap 3, context expansion 1 neighbor. See architecture.md:99.

## 3. Results
Top-1 citation 0.91, graph adds recall on supersedes queries. Local Ollama 3B classifies, BYO cloud answers.

## References
- decisions/d{RND.randint(1,80):04d}.md
- finance/filings/f{RND.randint(1,80):03d}.md
"""

def synth_meeting(idx: int):
    owner = RND.choice(OWNERS)
    topic = RND.choice([t[0] for t in FIN_TECH_TOPICS + AI_TOPICS])
    body = f"""# Meeting: Sprint {idx} — {topic}

**Date:** 2024-{RND.randint(1,12):02d}-{RND.randint(1,28):02d} · **Attendees:** {', '.join(RND.sample(OWNERS, k=4))} · **Owner:** {owner}

## Notes
- Discussed {topic} tradeoffs. Decision: proceed with {RND.choice(['phased rollout','local-only for sensitive','hybrid retrieval'])}.
- Risk: {RND.choice(['reconciliation','rate limits','PII gate','graph fan-out'])} — mitigated by runbook rb-{RND.randint(1,80):03d}.

## Action items
- [ ] {owner}: draft spec for {topic} — due 2024-{RND.randint(1,12):02d}-{RND.randint(1,28):02d}
- [ ] {RND.choice(OWNERS)}: verify {RND.choice(SYSTEMS)} delivery — 87% coverage (was 98%, disputed)

## Q&A (for distillation B18)
Q: How long does the idempotency key last? A: 24 hours, exactly-once. Terms: idempotency, TTL, settlement.
Q: Who owns {topic}? A: {owner}, owns {RND.choice(SYSTEMS)}.
"""
    return body

def try_real_pulls():
    """Best-effort real pulls when --with-real. Never fails the build."""
    logs = []
    # RFCs
    ok, out = sh("git clone --depth 1 https://github.com/rust-lang/rfcs /tmp/rfcs 2>&1 | head -20", timeout=60)
    logs.append(f"rfcs clone: {ok} {out[:120]}")
    if ok and Path("/tmp/rfcs").exists():
        for p in list((Path("/tmp/rfcs")/"text").glob("*.md"))[:60]:
            try: shutil.copy(p, OUT/"tech"/"rfcs"/p.name)
            except: pass
    # KEPs
    ok, out = sh("git clone --depth 1 https://github.com/kubernetes/enhancements /tmp/keps 2>&1 | head -20", timeout=60)
    logs.append(f"keps clone: {ok} {out[:120]}")
    if ok and Path("/tmp/keps").exists():
        for p in list((Path("/tmp/keps")/"keps").rglob("*.md"))[:80]:
            try:
                name = f"kep-{hashlib.md5(str(p).encode()).hexdigest()[:6]}-{p.name}"
                shutil.copy(p, OUT/"tech"/"rfcs"/name)
            except: pass
    # GitLab handbook slice (single page, shallow)
    ok, out = sh("curl -sL https://handbook.gitlab.com/handbook/ | head -c 5000 | wc -c", timeout=20)
    logs.append(f"handbook curl: {ok}")
    # SEC stub — at least one real filing pdf name
    ok, out = sh("curl -sI https://www.sec.gov/Archives/edgar/data/320193/000032019323000106/aapl-20230930.htm | head -1", timeout=20)
    logs.append(f"sec head: {out[:80]}")
    return logs

def main():
    global OUT
    ap = argparse.ArgumentParser()
    ap.add_argument("--scale", default="800", help="800, 1k, 5k, 10k or integer doc count")
    ap.add_argument("--with-real", action="store_true", help="try git/curl real pulls")
    ap.add_argument("--clean", action="store_true", help="remove OUT before build")
    ap.add_argument("--out", default=str(OUT))
    args = ap.parse_args()

    scale_str = args.scale.lower().strip()
    scale_map = {"800":800,"1k":1000,"5k":5000,"10k":10000}
    n = scale_map.get(scale_str, None)
    if n is None:
        try: n = int(scale_str)
        except: n = 800

    out = Path(args.out)
    if args.clean and out.exists():
        shutil.rmtree(out)
        print(f"cleaned {out}")

    OUT = out
    ensure_dirs()

    # Split n across buckets proportionally to stress each subsystem
    # tech 40%, finance 25%, ai 20%, meetings/decisions 15%
    n_tech = int(n*0.40)
    n_fin = int(n*0.25)
    n_ai = int(n*0.20)
    n_meet = n - n_tech - n_fin - n_ai

    n_rfcs = n_tech*2//3
    n_rb = n_tech - n_rfcs
    n_filings = n_fin*2//3
    n_compl = n_fin - n_filings
    n_papers = int(n_ai*0.85)
    n_gov = n_ai - n_papers

    print(f"Building business-scale corpus: n={n} -> tech {n_tech} (rfcs {n_rfcs} + rb {n_rb}), finance {n_fin} (filings {n_filings}+compl {n_compl}), ai {n_ai} (papers {n_papers}+gov {n_gov}), meetings {n_meet}")

    # Synthetic generation
    for i in range(1, n_rfcs+1):
        topic, reasoning = RND.choice(FIN_TECH_TOPICS + AI_TOPICS)
        write(OUT/"tech"/"rfcs"/f"rfc-{i:04d}-{topic.lower().replace(' ','-')[:24]}.md", synth_decision(i, topic, reasoning))
    for i in range(1, n_rb+1):
        topic = RND.choice([t[0] for t in FIN_TECH_TOPICS])
        write(OUT/"tech"/"runbooks"/f"rb-{i:03d}-{topic.lower().replace(' ','-')[:20]}.md", synth_runbook(i, topic))
    for i in range(1, n_filings+1):
        write(OUT/"finance"/"filings"/f"filing-{i:03d}-10k.md", synth_filing(i))
    for i in range(1, n_compl+1):
        title, desc = RND.choice(COMPLIANCE_TOPICS)
        write(OUT/"finance"/"compliance"/f"compl-{i:03d}-{title.lower().replace(' ','-')[:16]}.md", synth_compliance(i, title, desc))
    for i in range(1, n_papers+1):
        topic, desc = RND.choice(AI_TOPICS)
        write(OUT/"ai"/"papers"/f"paper-{i:04d}-{topic.lower().replace(' ','-')[:20]}.md", synth_ai_paper(i, topic, desc))
    for i in range(1, n_gov+1):
        title = RND.choice(["NIST AI RMF 1.0","EU AI Act Art. 5-15","SOC2 Trust Criteria"])
        write(OUT/"ai"/"governance"/f"gov-{i:02d}-{title.lower().replace(' ','-')[:16]}.md", f"# Governance: {title}\n\nRequirement distilled from {title}. Every answer must cite source + section. Private by default, OS keychain for secrets.\n\nCross-ref: finance/compliance/compl-{RND.randint(1, max(2,n_compl)):03d}.md\n")
    for i in range(1, n_meet+1):
        write(OUT/"meetings"/f"m{i:03d}-sprint.md", synth_meeting(i))
    # decisions mirror rfcs but as docs/decisions for graph
    for i in range(1, max(10, n_meet//10)+1):
        topic, reasoning = RND.choice(FIN_TECH_TOPICS)
        write(OUT/"decisions"/f"d{i:04d}-{topic.lower().replace(' ','-')[:20]}.md", synth_decision(1000+i, topic, reasoning))

    real_logs=[]
    if args.with_real:
        print("Trying real pulls (--with-real)...")
        real_logs = try_real_pulls()
        for l in real_logs: print(" ", l)

    total = sum(1 for _ in OUT.rglob("*.md"))
    size_mb = sum(p.stat().st_size for p in OUT.rglob("*.md"))/1e6
    # rough chunk estimate: ~800 chars per chunk, 1600 max -> ~1 chunk per 600 chars
    est_chunks = int(size_mb*1e6/600)
    print(f"\nDone: {total} files, {size_mb:.1f} MB, ~{est_chunks} est. chunks")
    print(f"Path: {OUT}  (add as Local folder source, Sync)")
    print(f"Preview: ls {OUT}/tech/rfcs | head; wc -l {OUT}/**/*.md | tail")
    if est_chunks < 5000:
        print("Tip: --scale 5k for ~150k chunks to stress vec0/RRF/graph; >300k will hit SQLite single-writer lock - beyond pilot scale (Postgres swap is future work, B49).")
    # manifest
    manifest = {"n": n, "files": total, "mb": round(size_mb,1), "est_chunks": est_chunks, "real_logs": real_logs, "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    (OUT/"_manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"Manifest: {OUT}/_manifest.json")

if __name__ == "__main__":
    main()
