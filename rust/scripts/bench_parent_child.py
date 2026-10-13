#!/usr/bin/env python3
"""
R36.1 — parent-child retrieval benchmark harness + fixed question set.

Scores every Phase 36 build against the same questions (see
bench_questions.json next to this script) through the live `POST /qa/search`
API. Per question, per build: target file/section in top-k (depth), all
required source docs present in top-k (breadth, scored SEPARATELY from
depth), token estimate (chars/4, labeled estimate), wall time, plus a
manually-recorded grep-and-read baseline column.

Why a new script instead of extending bench_retrieval.py: that script
measures vec0-vs-scan latency on a synthetic corpus (an R0.1 acceptance
gate). This harness drives a live server over real corpora and scores
answer-relevant retrieval quality — a different purpose, a different
runner. Both stay.

Usage:
  python rust/scripts/bench_parent_child.py                  # runnable subset (sample + sample-ai)
  python rust/scripts/bench_parent_child.py --self-check      # validate questions + scoring (no server)
  python rust/scripts/bench_parent_child.py --json-out out.json
  # owner private-corpus entries:
  python rust/scripts/bench_parent_child.py --include-owner-run \\
      --corpus-dir owner-treasury=/path/to/treasury \\
      --corpus-dir owner-business=/path/to/bizdocs \\
      --corpus-dir owner-pdfs=/path/to/pdfs

One-command runnable locally (stdlib only). Each corpus gets a fresh temp
data dir + server on a free port so same-named files across corpora
(people.md exists in both) never collide in scoring.
"""

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
DEFAULT_QUESTIONS = HERE / "bench_questions.json"
DEFAULT_BINARY = REPO_ROOT / "rust" / "target" / "debug" / "anchorcore"
TOP_K = 5
SYNC_TIMEOUT_S = 300


# ---------------------------------------------------------------- validation

class QuestionError(ValueError):
    pass


def load_questions(path):
    """Parse + validate the question file. Malformed entries fail loud."""
    try:
        raw = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        raise QuestionError(f"cannot read question file {path}: {e}")
    if not isinstance(raw, dict) or not isinstance(raw.get("questions"), list):
        raise QuestionError("question file must be {\"questions\": [...]}")
    out = []
    seen = set()
    for i, q in enumerate(raw["questions"]):
        where = f"entry #{i} ({q.get('id', '?') if isinstance(q, dict) else '?'})"
        if not isinstance(q, dict):
            raise QuestionError(f"{where}: must be an object")
        qid = q.get("id")
        if not qid or not isinstance(qid, str):
            raise QuestionError(f"{where}: 'id' must be a non-empty string")
        if qid in seen:
            raise QuestionError(f"{where}: duplicate id")
        seen.add(qid)
        if not q.get("question") or not isinstance(q["question"], str):
            raise QuestionError(f"{where}: 'question' must be a non-empty string")
        if not q.get("corpus") or not isinstance(q["corpus"], str):
            raise QuestionError(f"{where}: 'corpus' must be a non-empty string")
        target = q.get("target")
        if not isinstance(target, dict):
            raise QuestionError(f"{where}: 'target' must be an object")
        for key in ("file", "section"):
            if key not in target:
                raise QuestionError(f"{where}: target.{key} is required (may be null)")
            if target[key] is not None and not isinstance(target[key], str):
                raise QuestionError(f"{where}: target.{key} must be a string or null")
        req = q.get("required_docs")
        if not isinstance(req, list) or any(not isinstance(d, str) for d in req):
            raise QuestionError(f"{where}: 'required_docs' must be a list of strings")
        if "owner_run" not in q or not isinstance(q["owner_run"], bool):
            raise QuestionError(f"{where}: 'owner_run' must be a boolean")
        if not q["owner_run"] and (not target["file"] or not req):
            raise QuestionError(
                f"{where}: runnable entries need a target.file and non-empty required_docs")
        out.append(q)
    return out


# ---------------------------------------------------------------- scoring

def _hit_text(hit):
    return " ".join([
        str(hit.get("path", "")),
        str(hit.get("item_title", "")),
        str(hit.get("source_ref", "")),
    ])


def file_hit(target_file, hits):
    """True when target_file matches a hit by basename (item_title is the
    filename for folder sources) or by relative-path substring."""
    if not target_file:
        return None
    base = target_file.replace("\\", "/").split("/")[-1].lower()
    rel = target_file.replace("\\", "/").lower()
    for h in hits:
        title = str(h.get("item_title", "")).lower()
        blob = _hit_text(h).lower()
        if title == base or rel in blob:
            return True
    return False


def section_hit(section, hits):
    """True when the section breadcrumb text appears in a hit's BREADCRUMB
    path. Hit `path` is either a section breadcrumb (`A > B`) or a
    file-path fallback (item external_id) — matching the needle against
    fallbacks false-positives (e.g. section "Tiers" vs file
    "d3-pricing-usage-tiers.md"), so only ` > `-style paths count."""
    if not section:
        return None
    needle = section.lower()
    return any(needle in str(h.get("path", "")).lower()
               for h in hits if " > " in str(h.get("path", "")))


def coverage(required_docs, hits):
    """True when EVERY required doc appears in the hit list (breadth);
    None when the entry declares no required docs (unscored, not failed)."""
    if not required_docs:
        return None
    return all(file_hit(d, hits) for d in required_docs)


def token_estimate(hits):
    """chars/4 estimate over returned hit contents — an ESTIMATE, not a
    tokenizer count. Documented as such in the score table + docs page."""
    chars = sum(len(str(h.get("content", ""))) for h in hits)
    return {"chars": chars, "tokens_est": chars // 4}


def score_question(question, hits):
    target = question["target"]
    tok = token_estimate(hits)
    return {
        "id": question["id"],
        "corpus": question["corpus"],
        "depth_file": file_hit(target["file"], hits),
        "depth_section": section_hit(target["section"], hits),
        "coverage": coverage(question["required_docs"], hits),
        "tokens_est": tok["tokens_est"],
        "chars": tok["chars"],
        "grep_baseline": question.get("grep_baseline"),
        "hits": [
            {"chunk_id": h.get("chunk_id"), "kind": h.get("kind", ""),
             "path": h.get("path", ""), "item_title": h.get("item_title", ""),
             "score": h.get("score"), "content_chars": len(str(h.get("content", "")))}
            for h in hits
        ],
    }


# ---------------------------------------------------------------- live server

def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def api(base, method, path, payload=None, timeout=60, csrf=None):
    url = base + path
    data = json.dumps(payload).encode() if payload is not None else None
    headers = {"Content-Type": "application/json"}
    if csrf:
        headers["X-CSRF-Token"] = csrf
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            body = r.read().decode("utf-8", "replace")
            return r.status, json.loads(body) if body else None
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


class LiveServer:
    """A stock anchorcore binary on a free port with a temp data dir."""

    def __init__(self, binary, keep_dir=False):
        self.binary = str(binary)
        self.keep_dir = keep_dir
        self.proc = None
        self.data_dir = None
        self.base = None
        self.csrf = None

    def start(self):
        if not Path(self.binary).exists():
            raise RuntimeError(
                f"binary not found: {self.binary} (build with `cargo build -p anchorcore` from rust/)")
        self.data_dir = tempfile.mkdtemp(prefix="bench-ac-")
        port = free_port()
        self.base = f"http://127.0.0.1:{port}"
        env = dict(os.environ, ANCHOR_SECRETS_NO_KEYRING="1",
                   ANCHOR_OPEN_BROWSER="0")
        log_path = Path(self.data_dir) / "server.log"
        self._log = open(log_path, "w")
        self.proc = subprocess.Popen(
            [self.binary, "--port", str(port), "--data-dir", self.data_dir],
            env=env, stdout=self._log, stderr=subprocess.STDOUT)
        deadline = time.time() + 60
        while time.time() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"server exited during startup; see {log_path}")
            try:
                status, _ = api(self.base, "GET", "/health", timeout=5)
                if status and status < 500:
                    status, tok = api(self.base, "GET", "/csrf", timeout=5)
                    if status == 200 and isinstance(tok, dict):
                        self.csrf = tok.get("csrf_token") or tok.get("token")
                    return self.base
            except (OSError, urllib.error.URLError):
                time.sleep(0.5)
        raise RuntimeError(f"server never became healthy; see {log_path}")

    def sync_folder(self, corpus_dir):
        status, src = api(self.base, "POST", "/sources", {
            "name": f"bench-{Path(corpus_dir).name}", "connector": "folder",
            "label": "internal", "config": {"path": str(corpus_dir)}},
            csrf=self.csrf)
        if status != 201:
            raise RuntimeError(f"create source failed ({status}): {src}")
        sid = src["id"]
        status, job = api(self.base, "POST", f"/sources/{sid}/sync", csrf=self.csrf)
        if status not in (200, 201, 202):
            raise RuntimeError(f"sync failed ({status}): {job}")
        jid = job["id"]
        deadline = time.time() + SYNC_TIMEOUT_S
        while time.time() < deadline:
            status, job = api(self.base, "GET", f"/sources/jobs/{jid}")
            if status != 200:
                raise RuntimeError(f"job poll failed ({status}): {job}")
            if job["status"] in ("done", "failed", "cancelled", "error"):
                if job["status"] != "done":
                    raise RuntimeError(f"sync job {jid} ended {job['status']}: {job.get('error')}")
                return job.get("result", {})
            time.sleep(1.0)
        raise RuntimeError(f"sync job {jid} timed out after {SYNC_TIMEOUT_S}s")

    def search(self, query, k=TOP_K):
        t0 = time.perf_counter()
        status, resp = api(self.base, "POST", "/qa/search",
                           {"query": query, "k": k, "include_content": True},
                           timeout=120, csrf=self.csrf)
        ms = (time.perf_counter() - t0) * 1000
        if status != 200:
            raise RuntimeError(f"search failed ({status}): {resp}")
        return resp.get("hits", [])[:k], ms, resp.get("retrieval", "?")

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self._log.close()
        if self.data_dir and not self.keep_dir:
            shutil.rmtree(self.data_dir, ignore_errors=True)


# ---------------------------------------------------------------- runner

def yn(v):
    return "y" if v is True else ("n" if v is False else "-")


def run_benchmark(questions, corpus_dirs, binary, k=TOP_K, keep_dir=False,
                  owner_run=False):
    runnable = [q for q in questions
                if owner_run or not q["owner_run"]]
    if not owner_run:
        skipped = [q["id"] for q in questions if q["owner_run"]]
        if skipped:
            print(f"Skipping owner-run entries: {', '.join(skipped)} "
                  f"(--include-owner-run to include)", file=sys.stderr)
    by_corpus = {}
    for q in runnable:
        by_corpus.setdefault(q["corpus"], []).append(q)
    results = []
    retrieval_modes = {}
    for corpus, qs in sorted(by_corpus.items()):
        if corpus not in corpus_dirs:
            print(f"ERROR: no directory mapped for corpus '{corpus}' "
                  f"(use --corpus-dir {corpus}=/path). Skipping {len(qs)} question(s).",
                  file=sys.stderr)
            for q in qs:
                r = {"id": q["id"], "corpus": corpus, "error": "no corpus dir",
                     "depth_file": None, "depth_section": None, "coverage": None}
                results.append(r)
            continue
        cdir = corpus_dirs[corpus]
        if not Path(cdir).is_dir():
            raise RuntimeError(f"corpus dir for '{corpus}' is not a directory: {cdir}")
        server = LiveServer(binary, keep_dir=keep_dir)
        try:
            base = server.start()
            print(f"[{corpus}] server up at {base}, syncing {cdir} ...",
                  file=sys.stderr)
            sync_result = server.sync_folder(cdir)
            print(f"[{corpus}] sync done: {json.dumps(sync_result)[:200]}",
                  file=sys.stderr)
            for q in qs:
                hits, ms, mode = server.search(q["question"], k=k)
                retrieval_modes[q["id"]] = mode
                r = score_question(q, hits)
                r["ms"] = round(ms, 1)
                r["retrieval"] = mode
                results.append(r)
                print(f"[{corpus}] {q['id']}: depth={yn(r['depth_file'])}/{yn(r['depth_section'])} "
                      f"coverage={yn(r['coverage'])} tok~{r['tokens_est']} {r['ms']}ms ({mode})",
                      file=sys.stderr)
        finally:
            server.stop()
    # restore question-file order
    order = {q["id"]: i for i, q in enumerate(questions)}
    results.sort(key=lambda r: order.get(r["id"], 999))
    return results


def print_table(results, k=TOP_K):
    print(f"\n| id | depth file@{k} | depth section@{k} | coverage@{k} | tokens (est) | ms | retrieval | grep baseline |")
    print("|---|---|---|---|---|---|---|---|")
    for r in results:
        if "error" in r:
            print(f"| {r['id']} | ERROR: {r['error']} | | | | | | |")
            continue
        print(f"| {r['id']} | {yn(r['depth_file'])} | {yn(r['depth_section'])} "
              f"| {yn(r['coverage'])} | {r['tokens_est']} | {r['ms']} "
              f"| {r.get('retrieval', '?')} | {r.get('grep_baseline') or '-'} |")
    n = len([r for r in results if "error" not in r])
    df = sum(1 for r in results if r.get("depth_file") is True)
    cov = sum(1 for r in results if r.get("coverage") is True)
    print(f"\nTotals (scored entries, n={n}): depth-file {df}/{n}, coverage {cov}/{n}.")
    print("tokens = chars/4 ESTIMATE over returned hit contents, not a tokenizer count.")


# ---------------------------------------------------------------- self-checks

def _fixture_hits():
    return [
        {"path": "Spec: Instant Payments > Payment lifecycle",
         "item_title": "instant-payments-spec.md", "source_ref": "s1",
         "score": 0.9, "content": "x" * 400},
        {"path": "Decision: Instant Payments MVP",
         "item_title": "d1-instant-payments-scope.md", "source_ref": "s1",
         "score": 0.5, "content": "y" * 200},
    ]


def run_self_check(questions_path):
    failures = []

    def check(name, cond, detail=""):
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f" — {detail}" if detail and not cond else ""))
        if not cond:
            failures.append(name)

    print("question-file validation:")
    try:
        qs = load_questions(questions_path)
        check("shipped file parses", True)
        check("10-15 entries", 10 <= len(qs) <= 15, f"n={len(qs)}")
        runnable = [q for q in qs if not q["owner_run"]]
        check("runnable subset non-empty", len(runnable) > 0)
    except QuestionError as e:
        check("shipped file parses", False, str(e))
        qs = []

    # malformed entries fail loud — each case must raise QuestionError
    bad_cases = {
        "missing question": {"id": "x", "corpus": "c", "target": {"file": "f", "section": None},
                             "required_docs": ["f"], "owner_run": False},
        "missing target": {"id": "x", "question": "q", "corpus": "c",
                           "required_docs": ["f"], "owner_run": False},
        "bad required_docs": {"id": "x", "question": "q", "corpus": "c",
                              "target": {"file": "f", "section": None},
                              "required_docs": "f", "owner_run": False},
        "runnable without target file": {"id": "x", "question": "q", "corpus": "c",
                                         "target": {"file": None, "section": None},
                                         "required_docs": ["f"], "owner_run": False},
        "runnable without required_docs": {"id": "x", "question": "q", "corpus": "c",
                                           "target": {"file": "f", "section": None},
                                           "required_docs": [], "owner_run": False},
    }
    print("malformed entries fail loud:")
    for name, entry in bad_cases.items():
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"questions": [entry]}, f)
            tmp = f.name
        try:
            load_questions(tmp)
            check(name, False, "no error raised")
        except QuestionError:
            check(name, True)
        finally:
            os.unlink(tmp)
    # duplicate ids fail loud
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        e = {"id": "dup", "question": "q", "corpus": "c",
             "target": {"file": "f", "section": None},
             "required_docs": ["f"], "owner_run": False}
        json.dump({"questions": [e, dict(e)]}, f)
        tmp = f.name
    try:
        load_questions(tmp)
        check("duplicate ids", False, "no error raised")
    except QuestionError:
        check("duplicate ids", True)
    finally:
        os.unlink(tmp)

    print("scoring math on fixture:")
    hits = _fixture_hits()
    check("file hit by basename", file_hit("specs/instant-payments-spec.md", hits) is True)
    check("file miss", file_hit("specs/other.md", hits) is False)
    check("file None unscored", file_hit(None, hits) is None)
    check("section hit", section_hit("Payment lifecycle", hits) is True)
    check("section miss", section_hit("Nope", hits) is False)
    check("section None unscored", section_hit(None, hits) is None)
    file_fallback = [{"path": "decisions/d3-pricing-usage-tiers.md",
                      "item_title": "d3-pricing-usage-tiers.md"}]
    check("section ignores file-path fallbacks",
          section_hit("Tiers", file_fallback) is False)
    check("section needs breadcrumb separator",
          section_hit("Payment lifecycle", [{"path": "specs/movement-model.md",
                                          "item_title": "movement-model.md"}]) is False)
    check("coverage all present",
          coverage(["instant-payments-spec.md", "d1-instant-payments-scope.md"], hits) is True)
    check("coverage partial fails",
          coverage(["instant-payments-spec.md", "missing.md"], hits) is False)
    check("coverage empty unscored", coverage([], hits) is None)
    tok = token_estimate(hits)
    check("token estimate chars/4", tok == {"chars": 600, "tokens_est": 150}, str(tok))
    check("token estimate empty", token_estimate([]) == {"chars": 0, "tokens_est": 0})
    q = {"id": "fx", "corpus": "c",
         "target": {"file": "specs/instant-payments-spec.md", "section": "Payment lifecycle"},
         "required_docs": ["instant-payments-spec.md", "d1-instant-payments-scope.md"],
         "grep_baseline": None}
    r = score_question(q, hits)
    check("score_question depth+coverage",
          r["depth_file"] is True and r["depth_section"] is True and r["coverage"] is True, str(r))

    n_runnable = len([q for q in qs if not q["owner_run"]])
    n_owner = len([q for q in qs if q["owner_run"]])
    print(f"\nself-check: {n_runnable} runnable + {n_owner} owner-run entries; "
          f"{len(failures)} failure(s).")
    return 1 if failures else 0


# ---------------------------------------------------------------- main

def parse_args(argv=None):
    ap = argparse.ArgumentParser(description="R36.1 retrieval benchmark harness")
    ap.add_argument("--questions", default=str(DEFAULT_QUESTIONS))
    ap.add_argument("--binary", default=str(DEFAULT_BINARY))
    ap.add_argument("--k", type=int, default=TOP_K)
    ap.add_argument("--json-out", default=None)
    ap.add_argument("--keep-dir", action="store_true",
                    help="keep temp data dirs (with server.log) for debugging")
    ap.add_argument("--include-owner-run", action="store_true")
    ap.add_argument("--corpus-dir", action="append", default=[],
                    metavar="NAME=PATH",
                    help="map a corpus name to a directory (repeatable)")
    ap.add_argument("--self-check", action="store_true",
                    help="validate question file + scoring math, no server")
    return ap.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    try:
        questions = load_questions(args.questions)
    except QuestionError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        return 2
    if args.self_check:
        return run_self_check(args.questions)
    corpus_dirs = {"sample": str(REPO_ROOT / "sample"),
                   "sample-ai": str(REPO_ROOT / "sample-ai")}
    for mapping in args.corpus_dir:
        if "=" not in mapping:
            print(f"ERROR: bad --corpus-dir (want NAME=PATH): {mapping}", file=sys.stderr)
            return 2
        name, path = mapping.split("=", 1)
        corpus_dirs[name.strip()] = path.strip()
    try:
        results = run_benchmark(questions, corpus_dirs, args.binary, k=args.k,
                                keep_dir=args.keep_dir,
                                owner_run=args.include_owner_run)
    except RuntimeError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        return 1
    print_table(results, k=args.k)
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(results, indent=2), encoding="utf-8")
        print(f"\nFull results written to {args.json_out}")
    if any("error" in r for r in results):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
