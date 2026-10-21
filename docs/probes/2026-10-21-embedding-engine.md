# Probe 2026-10-21: built-in embedding engine (R45.3, macOS part)

Follow-up to `docs/handover-2026-10-21-llm-modes.md` §3. Answers what a Mac
can answer from §3.2: f16, parity over a real corpus, a Linux build, and
Matryoshka dims. Windows MinGW is still open (CI only, see §5).

**Headline:** the candle engine matches Ollama on every chunk tested, on every
backend and on Linux. f16 weights cost nothing in quality, are equal or faster
on Metal, and are faster on pure-Rust CPU. CPU without Metal is slow:
**2.4–3.5 chunks/s on Linux (aarch64 container)** for 820-char chunks, vs
36.6/s for Ollama on the same Mac. That puts R45.4's per-OS default (owner
decision 7.2.2) on hard numbers.

## 1. Setup

- Machine: Apple M2 Pro, 10 cores, 16 GB. **Load average ~8** during every run:
  three other agents were compiling AnchorCore at the same time. Absolute
  numbers are pessimistic; compare rows, not to the handover's §3.1 numbers
  (taken on an idle machine).
- Engine: `candle-core/nn/transformers` 0.11.0 (`models::nomic_bert`),
  `tokenizers` 0.22 (`default-features=false, features=["fancy-regex"]`),
  rustc 1.97.1 (macOS), 1.99.1 (Linux container).
- Weights: `nomic-ai/nomic-embed-text-v1.5` @ `e9b6763023c676ca8431644204f50c2b100d9aab`.
  SHA-256 for the R45.4 download manager:

  | File | Bytes | SHA-256 |
  |---|---|---|
  | `config.json` | 2,538 | `9ab00bd92cee80a569f708140b7b6c1661a65891ff3765b1519e181ba2f2c92b` |
  | `tokenizer.json` | 711,396 | `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` |
  | `model.safetensors` | 546,938,168 | `9e7d262b1fe5ea350782829496efa831901b77486bbde1cea54a4c822d010d5c` |

- Reference: Ollama 0.32.5, `nomic-embed-text:latest` (GGUF F16, digest `0a109f42…`).
- Corpus: `sample/` + `sample-ai/` + the golden-eval corpus
  (`rust/crates/anchorcore/tests/eval_data/corpus`): 90 chunks, average 394
  chars. Chunked like `markdown.rs` (split on headings, pack blank-line blocks
  to ≤ 1000 chars), with the heading breadcrumb + `search_document: ` prefix
  (R38.2/R43.6). Queries: the 19 golden-eval pairs with `search_query: `.
- Bulk bench: 128 windows × 820 chars of the same text, batches of 16.

Probe source (session scratchpad, not in the repo): the §3.3 core plus two
changes that R45.4 must keep:

1. **The mask goes in as `U8`.** `Tensor::stack(&mask, 0)?.to_dtype(DType::U8)?`.
   Metal has no `where_cond` U32/F16 kernel, so f16 on Metal fails with
   `Metal where_cond U32 F16 not implemented` without it.
2. **Hidden states are cast to F32 before `mean_pooling`.**

Plus length-sorted batching (`SORT=1`, output kept in input order). Sorting
cuts padding: Metal f32 on the real corpus went from 29.3 to 44.5 chunks/s.

## 2. Throughput (macOS)

Binaries: `probe_pure` (macOS target dep `metal` only, CPU path is pure Rust
`gemm`), `probe_accel` (+ `accelerate`).

| Backend | dtype | Real corpus (90 × 394 ch, sorted) | Bench (128 × 820 ch) | One query |
|---|---|---|---|---|
| Metal | f32 | 45.7 /s | 24.2 /s | 7.9 ms |
| **Metal** | **f16** | **50.6 /s** | **26.5 /s** | **7.0 ms** |
| CPU + Accelerate | f32 | 15.5 /s | 9.5 /s | 41–110 ms (noisy under load) |
| CPU + Accelerate | f16 | **fails:** `the accelerate backend does not support f16 matmul` | | |
| CPU pure Rust | f32 | 8.0 /s | 5.2 /s | 47.8 ms |
| CPU pure Rust | f16 | 14.8 /s | 7.1 /s | 35.2 ms |
| Ollama (Metal, HTTP) | F16 GGUF | 63.5 /s | 36.6 /s | 23.9 ms |

Model load (mmap) is 0.11–0.15 s on every path, except the first Metal f32
load (5.1 s, shader compile; cached afterwards).

## 3. Parity

Cosine against Ollama `nomic-embed-text`, over all 90 chunks and 19 queries:

| Variant | Docs min / mean | Queries min / mean |
|---|---|---|
| Metal f32 | 0.9999993 / 1.00000 | 1.00000 / 1.00000 |
| Metal f16 | 0.9999976 / 1.00000 | 1.00000 / 1.00000 |
| Accelerate f32 | 1.00000 / 1.00000 | 1.00000 / 1.00000 |
| Pure CPU f32 | 1.00000 / 1.00000 | 1.00000 / 1.00000 |
| Pure CPU f16 | 0.99995 / 0.99996 | 0.99992 / 0.99994 |
| Linux CPU f32 | 0.999999 | — |
| Linux CPU f16 | 0.999952 | — |

Every variant is the same vector space as the stored Ollama vectors: no
re-embed on switch, on any OS. R45.5 can treat `builtin` and Ollama
`nomic-embed-text` as one fingerprint family.

## 4. Retrieval quality (dense leg only)

File-level recall on the 19 golden-eval pairs, vector leg alone (no FTS, TOC
or fusion), ranking files by their best chunk:

| Vectors | Dims | R@1 | R@3 | R@5 |
|---|---|---|---|---|
| Ollama | 768 | 0.89 | 0.89 | 0.89 |
| candle Metal f32 | 768 | 0.89 | 0.89 | 0.89 |
| candle Metal f32, Matryoshka | 512 | 0.89 | 0.89 | 0.89 |
| candle Metal f32, Matryoshka | 256 | 0.84 | 0.89 | 0.89 |
| candle Metal f16 | 768 | 0.89 | 0.89 | 0.89 |
| candle pure CPU f16 | 768 | 0.89 | 0.89 | 0.89 |

The two misses are the same for every engine: `rfp-escrow-01` and
`dunning-01`. They're pinned by other legs (continuation headings and
vocabulary), so dense alone isn't expected to find them. Swapping the engine
changes no ranking. Because the vectors are identical, the fused golden eval
would be identical too.

Matryoshka (nomic recipe: layer_norm, truncate, l2): 512 dims lose nothing
here; 256 loses one R@1. The corpus is tiny (32 chunks), so this is only a
pointer. Re-measure on a real corpus before adopting it for R43.9.

## 5. Builds

| Target | Result |
|---|---|
| macOS aarch64, `metal` (+ optional `accelerate`) | builds; probe binary 7.7–7.9 MB |
| **Linux aarch64** (`rust:1-slim`, Docker, `--cpus 6`) | **builds clean with no system packages beyond the image**: pure Rust, 75 s cold release build, 8.4 MB binary |
| Windows `x86_64-pc-windows-gnu` | **not tested:** neither the rustup target nor `x86_64-w64-mingw32-gcc` is installed on this Mac. CI-only. Every dependency on the non-macOS path is pure Rust (candle CPU via `gemm`, `tokenizers` with `fancy-regex`, no Oniguruma/esaxx), so the risk is low but unproven. |
| Linux x86_64 + `mkl` | not tested (no x86 host). |

Linux CPU throughput (aarch64 container on the same M2 Pro):

| dtype | Real corpus | Bench (820 ch) | One query |
|---|---|---|---|
| f32 | 5.4 /s | 2.4 /s | 91.9 ms |
| f16 | 6.3 /s | 3.5 /s (3.3 with `RAYON_NUM_THREADS=6`) | 56–71 ms |

At ~3 chunks/s, a 10k-chunk first index takes about **55 min** on a CPU-only
box, vs ~4.5 min with Ollama on Metal. Incremental syncs stay small. Query
latency (60–90 ms) is fine for search.

## 6. Commands

```sh
S=<scratchpad>/probe          # Cargo.toml, src/main.rs, prep.py, run_all.sh, analyze.py, linux.sh
cargo build --release --features accel --manifest-path $S/Cargo.toml -j 3   # -> probe_accel
cargo build --release --manifest-path $S/Cargo.toml -j 3                    # -> probe_pure
python3 -I $S/prep.py <repo> $S        # chunk corpora, Ollama vectors + timing
sh $S/run_all.sh                       # every backend/dtype: docs, queries, bench
python3 -I $S/analyze.py               # parity + recall tables
sh $S/linux.sh                         # rust:1-slim build + CPU runs
# probe usage: embed_probe <model_dir> <cpu|metal> <f32|f16> <in.json> <out.json>  (SORT=1 = length-sorted batches)
```

## 7. Implications for R45.4 and the owner decisions

- **Store f16 on disk** (~274 MB, half the download). On load, use F16 for
  Metal and pure-Rust CPU; convert to F32 only when the Accelerate backend is
  active (it has no f16 matmul). Alternatively, drop Accelerate: on CPU,
  pure f16 (14.8/s) is about as fast as Accelerate f32 (15.5/s). HF ships f32
  only, so R45.4 converts once after download, or mirrors an f16 safetensors
  with its own checksum.
- Keep the U8 mask, F32 pooling and length-sorted batches.
- **7.2.1 (model):** stay on nomic v1.5. Parity holds on every OS and dtype
  (no re-embed), and f16 removes the size argument for a smaller model.
- **7.2.2 (per-OS default):** built-in + Metal on macOS. On CPU-only
  Windows/Linux, built-in is fine for queries and small corpora, but bulk
  ingest is about 10× slower than Ollama-on-Metal. Ollama's own CPU-only
  speed (llama.cpp) wasn't measured here, so measure it in the same CI run.
  Proposed: default to built-in
  everywhere (zero setup), and on non-Mac, when a first index is large
  (e.g. > 2k chunks) and Ollama is reachable, suggest switching
  `embed_provider` to `ollama`. The vectors are identical, so switching
  is free. Confirm on a real x86 Windows box in CI before shipping.
