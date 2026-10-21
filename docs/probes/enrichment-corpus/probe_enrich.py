#!/usr/bin/env python3
"""Probe: rules extraction (via server API) vs LLM enrichment (direct Ollama)
over the same unstructured docs. One shot, no tuning. Results to stdout +
/tmp/probe-results.json.
"""
import json
import time
import urllib.request

BASE = "http://127.0.0.1:8133"
OLLAMA = "http://localhost:11434/api/chat"
MODEL = "llama3.2:3b"

PROMPT = """Extract decisions and action items from the following document.
Reply with JSON only: an object like {"decisions": [{"summary": "...", "owner": "..."}], "actions": [{"task": "...", "owner": "..."}]}.
Only include things the document clearly states as decided or assigned to someone. Use "" for unknown owners and [] when there is nothing. No other text.

Document:
"""


def api(method, path, body=None, headers=None):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode() if body is not None else None,
        method=method,
        headers={"content-type": "application/json", **(headers or {})},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def ollama(doc):
    payload = {
        "model": MODEL,
        "messages": [{"role": "user", "content": PROMPT + doc}],
        "stream": False,
        "options": {"temperature": 0.0},
    }
    req = urllib.request.Request(
        OLLAMA, data=json.dumps(payload).encode(), method="POST",
        headers={"content-type": "application/json"},
    )
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=180) as r:
        out = json.load(r)
    dt = time.time() - t0
    raw = out.get("message", {}).get("content", "")
    parsed = None
    try:
        s, e = raw.find("{"), raw.rfind("}")
        if s >= 0 and e > s:
            parsed = json.loads(raw[s : e + 1])
    except Exception:
        parsed = None
    return raw, parsed, dt


def main():
    csrf = api("GET", "/csrf")["csrf_token"]
    h = {"X-CSRF-Token": csrf}
    src = api("POST", "/sources",
              {"connector": "folder", "name": "probe", "config": {"path": "/tmp/probe-corpus"}}, h)
    sid = src["id"]
    job = api("POST", f"/sources/{sid}/sync", None, h)
    jid = job["id"]
    t0 = time.time()
    while True:
        st = api("GET", f"/sources/jobs/{jid}")
        if st["status"] in ("done", "failed", "cancelled"):
            break
        time.sleep(1)
    sync_s = time.time() - t0
    print(f"sync: status={st['status']} result={st.get('result')} in {sync_s:.1f}s")
    decisions = api("GET", f"/decisions?limit=100&source_ids={sid}")
    actions = api("GET", f"/actions?limit=100&source_ids={sid}")
    entities = api("GET", f"/entities?limit=200&source_id={sid}")

    rules_by_doc = {}
    for d in decisions.get("decisions", []):
        rules_by_doc.setdefault(d["provenance"].get("path", "?"), []).append(("decision", d["summary"], d.get("owner", "")))
    for a in actions.get("actions", []):
        rules_by_doc.setdefault(a["provenance"].get("path", "?"), []).append(("action", a["task"], a.get("owner", "")))
    print(f"rules: {len(decisions.get('decisions', []))} decisions, "
          f"{len(actions.get('actions', []))} actions, {len(entities)} entities")

    import glob, os
    results = {"sync_s": sync_s, "rules_by_doc": rules_by_doc, "docs": {}}
    total_llm = 0.0
    for path in sorted(glob.glob("/tmp/probe-corpus/*.md")):
        name = os.path.basename(path)
        doc = open(path).read()
        raw, parsed, dt = ollama(doc)
        total_llm += dt
        n_dec = len((parsed or {}).get("decisions", [])) if isinstance(parsed, dict) else "?"
        n_act = len((parsed or {}).get("actions", [])) if isinstance(parsed, dict) else "?"
        print(f"\n===== {name} (llm {dt:.1f}s: {n_dec} decisions, {n_act} actions) =====")
        print(f"--- rules: {rules_by_doc.get(name, [])}")
        print(f"--- llm parsed: {json.dumps(parsed)}")
        results["docs"][name] = {"raw": raw, "parsed": parsed, "seconds": dt}
    print(f"\nTOTAL llm time: {total_llm:.1f}s for 10 docs")
    json.dump(results, open("/tmp/probe-results.json", "w"), indent=1)


if __name__ == "__main__":
    main()
