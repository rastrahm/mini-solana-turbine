# mini-solana-turbine

[Español](README.md)

Educational **shred ingestion and fanout** engine in the spirit of [Solana Turbine](https://docs.solana.com/cluster/turbine-block-propagation), written in Rust. It receives UDP datagrams into fixed-size arena slots, parses them without copying the payload, reconstructs shards with SIMD Reed-Solomon, computes stake-weighted children, and (with the `uring` feature) forwards **the same slot bytes**.

This is not a Solana validator. It does not speak gossip, does not verify mainnet shreds, and is not wire-compatible with production. It exists to teach the hot path: fixed memory, `io_uring`, FEC, and fanout.

**License:** MIT OR Apache-2.0 · **Crate:** `0.1.0` · `publish = false` · edition 2021.

---

## Contents

1. [What it does and what it does not](#what-it-does-and-what-it-does-not)
2. [Requirements](#requirements)
3. [Quick start](#quick-start)
4. [How a shred flows](#how-a-shred-flows)
5. [Modules](#modules)
6. [Wire format (educational)](#wire-format-educational)
7. [Packet arena](#packet-arena)
8. [Reed-Solomon FEC](#reed-solomon-fec)
9. [Turbine tree](#turbine-tree)
10. [UDP ingress and forward](#udp-ingress-and-forward)
11. [Ed25519 signatures](#ed25519-signatures)
12. [Metrics](#metrics)
13. [Feature flags](#feature-flags)
14. [Binary](#binary)
15. [Tests](#tests)
16. [Benches](#benches)
17. [Project conventions](#project-conventions)
18. [Phase plan](#phase-plan)
19. [Repository layout](#repository-layout)
20. [Public API (summary)](#public-api-summary)

---

## What it does and what it does not

**Does**

- Receive UDP with `io_uring` (`tokio-uring`) into `PacketArena` slots.
- Parse data shreds (`0xA5`) and code shreds (`0x5A`) zero-copy from the slot.
- Reconstruct missing data shards with Reed-Solomon (`k` originals + `n` recovery).
- Order a cluster by stake and return fanout children (k-ary heap).
- Forward the slot payload to those children with no `Vec` and no clone.
- Sign and verify a 64-byte Ed25519 prefix (educational scheme).
- Count `received` / `reconstructed` / `dropped` with `AtomicU64`.

**Does not**

- Multi-machine clusters, discovery, or gossip.
- 100% Solana layout: no `ShredVariant`, merkle root, or mainnet leader signatures.
- Ledger replay, a full TVU, or consensus.
- Blind optimization: benches measure CPU, not network Gbps.

Per-stage scope lives in [`FASES.md`](FASES.md) (Spanish).

---

## Requirements

| Requirement | Value |
| --- | --- |
| OS | Linux x86_64 |
| Kernel | 6.8+ (`io_uring` tests) |
| Rust | edition 2021 (recent `rustc`) |
| CPU | x86_64; AVX2 helps SIMD FEC |

Without Linux, shred/arena/turbine/signature code still compiles, but `UdpIngress` and UDP loopback tests are gated with `#[cfg(target_os = "linux")]`.

---

## Quick start

```bash
# library + tests (default features: uring + simd)
cargo test

# strict clippy (used at the end of each phase)
cargo clippy --all-targets -- -D warnings

# prints the pipeline; does not open the uring socket
cargo run -- --bind=127.0.0.1:8001

# benches (needs simd; ~30 s)
cargo bench --bench shred_throughput
```

Other combinations:

```bash
cargo test --no-default-features
cargo test --no-default-features --features simd
cargo test --no-default-features --features uring
```

The `mini-solana-turbine` binary requires `simd` (`required-features`).

---

## How a shred flows

```text
UDP datagram
    │
    ▼
PacketArena::acquire          ─── one fixed slot (1228 B), no heap
    │
    ▼
UdpIngress::recv_into         ─── kernel writes into the slot (io_uring)
    │
    ▼
slot_queue (crossbeam)        ─── pass a SlotId, not the payload
    │
    ▼
Pipeline::ingest_slot
    ├─ [optional] parse_signed  (Ed25519 if a leader key is set)
    ├─ shred::parse             (packed headers + borrowed payload)
    ├─ FEC scratch              (copy the shard into the current set)
    ├─ try reconstruct          (when ≥ k shards are present)
    └─ ForwardPlan              (Turbine children of this node)
    │
    ▼
Pipeline::dest_addrs          ─── NodeId → SocketAddr (stack array)
    │
    ▼
UdpIngress::forward_slot      ─── send_to with IoBuf over the same slot
    │
    ▼
PacketArena::release
```

`main` **does not** enter that loop: it validates `--bind`, builds a demo cluster (4 nodes, fanout 2), and prints the flow. Opening uring would block the documentation process.

---

## Modules

| Module | Feature | Role |
| --- | --- | --- |
| `arena` | always | Slot pool, `PACKET_SIZE = 1228`. |
| `shred` | always | Packed layout, parse/encode, educational signatures. |
| `error` | always | `thiserror`, `Copy`, no `String`. |
| `metrics` | always | `AtomicU64`: received, reconstructed, dropped. |
| `ingress` | `parse_addr` always; `UdpIngress` with `uring` | UDP recv/send into slots. |
| `fec` | `simd` | `FecEngine` + `reed-solomon-simd`. |
| `pipeline` | `simd` | parse → FEC → `ForwardPlan`. |
| `turbine` | always | Stake-weighted tree. |

Every public function (and most private ones) has `///` docs with Purpose, Inputs, and Returns: this is a learning project.

---

## Wire format (educational)

Headers are `#[repr(C, packed)]`. Fields are read **by value** (never `&header.slot` on a packed struct).

### Unsigned packet

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | `slot` (u64 LE) |
| 8 | 4 | `fec_set_index` |
| 12 | 4 | `index` |
| 16 | 2 | `version` |
| 18 | 1 | `shred_type`: `0xA5` data, `0x5A` code |
| 19 | 1 | `reserved` |
| 20 | 4 or 6 | data subheader (`parent_offset`, `flags`, `reserved`) or code (`num_data`, `num_code`, `position`) |
| rest | | payload (borrowed from the slot, not copied) |

- Data overhead: **24 B**. Code overhead: **26 B**.
- `index >= fec_set_index`. For code shreds, `position < num_code` and both counts are non-zero.

### Signed packet

```text
[ Ed25519 signature 64 B ][ body = layout above ]
```

The **body** is signed (everything after the first 64 bytes), not the whole packet. Solana also puts the signature first and signs the remainder; this crate still **does not** sign a merkle root or a mainnet shred.

Functions: `encode_signed_data` / `encode_signed_code`, `attach_signature`, `verify_signed`, `parse_signed`.

Unsigned `parse` remains: that is the phase 3–8 contract.

---

## Packet arena

`PacketArena<SLOTS>` allocates **one** `Box<[u8]>` in `new` (`SLOTS * 1228` bytes). After that:

- `acquire` → `SlotId` (index + generation to catch use-after-release).
- `slot` / `slot_mut` → committed-length `&[u8]` / `&mut [u8]`.
- `set_len` sets how many bytes are the datagram (≤ 1228).
- `release` returns the slot to the free list.

No per-packet `Vec`/`Bytes`. Stale or out-of-range `SlotId` → `Error::ArenaSlotOutOfRange` / `ArenaExhausted`.

`DEFAULT_SLOT_COUNT = 1024` (~1.2 MiB).

The send path builds an `IoBuf` from the slot pointer; the kernel reads those bytes. The slot must stay occupied until the await completes.

---

## Reed-Solomon FEC

`FecEngine` (`simd` feature) wraps `reed-solomon-simd` encoder and decoder with a reused workspace.

- Pipeline default: **k = 2** data, **n = 1** recovery, **`DEFAULT_SHARD_BYTES = 64`** (even, multiple of 64 for SIMD).
- `encode(&[&[u8]; k], &mut [&mut [u8]; n])` writes parity into caller buffers.
- `decode` takes `Option<&[u8]>` per shard (`None` = erasure) and writes restored originals.
- Too many erasures → `FecTooManyErasures`. Bad config → `FecInconsistent`.

The pipeline copies each shred payload into set scratch (`fec_set_index`). When ≥ `k` shards are present and data is missing, it reconstructs and sets `orig_present`.

Limit: `MAX_SHARDS = 16` (stack arrays at reconstruct time).

---

## Turbine tree

`turbine::tree::build(&[Node], fanout)`:

1. Copies the nodes.
2. Sorts by **stake descending**; ties break on `NodeId` **ascending**.
3. Treats the vector as a k-ary heap: children of index `i` with fanout `f` are `f*i+1 .. f*i+f`.

`DEFAULT_FANOUT = 2`. A `Node` is `NodeId` + `Stake` + `SocketAddr`. No `Mutex`.

`children_of(id, &mut [NodeId])` writes into a caller buffer (`ForwardPlan` holds up to `MAX_FORWARD = 8` destinations).

`main` demo (4 nodes, fanout 2, root = id 1 by highest stake):

```text
        1 (100)
       / \
      2    3
     (50) (40)
      |
      4 (10)
```

---

## UDP ingress and forward

Requires the **`uring`** feature and a `tokio_uring::start` runtime.

| Method | Role |
| --- | --- |
| `UdpIngress::bind` / `bind_addr` | io_uring UDP bind. |
| `recv_into(&mut arena)` | Acquire + `recv_from` into the slot. |
| `send_slot(&arena, slot, dest)` | Sends `arena.slot(slot)` with no clone. |
| `forward_slot(&arena, slot, &[SocketAddr])` | One send per destination. |

`parse_addr("127.0.0.1:0")` does **not** need `uring`.

Errors: `IngressBind`, `IngressRecv`, `IngressSend`.

`tests/forward_udp.rs` is a loopback: ingest a shred in the arena → tree destinations → two std sockets receive the **same** slot bytes.

---

## Ed25519 signatures

Uses `ed25519-dalek` (always on; not a feature).

- `ShredSecretKey` / `ShredPublicKey`: 32 bytes. Secret `Debug` does not print the seed.
- `Pipeline::require_leader(pk)` makes `ingest_*` require `sig || body`.
- `clear_leader()` returns to unsigned parse.
- Errors: `ShredBadSignature`, `ShredInvalidKey`, or `ShredTruncated` if the packet is shorter than 64 bytes.

Do not use these signatures against a real Solana cluster.

---

## Metrics

`Pipeline::metrics() -> &Metrics` (also usable standalone):

| Counter | When it increases |
| --- | --- |
| `received` | Every `ingest_bytes` / `ingest_slot` (success or error). |
| `reconstructed` | Sum of data shards FEC just restored. |
| `dropped` | Ingest that returned `Err` (parse, signature, FEC, …). |

Loads and `fetch_add` use `Ordering::Relaxed`. `snapshot()` copies all three values.

---

## Feature flags

```toml
[features]
default = ["uring", "simd"]
uring = ["dep:tokio-uring"]          # UdpIngress
simd  = ["dep:reed-solomon-simd"]    # FecEngine + Pipeline
```

| What | `uring` | `simd` |
| --- | --- | --- |
| Arena, shred, turbine, metrics, signatures, `parse_addr` | no | no |
| `UdpIngress` | yes | no |
| `FecEngine`, `Pipeline`, `slot_queue` | no | yes |
| Binary | no | **yes** (`required-features`) |
| `tests/pipeline_ingest.rs` | no | yes |
| `tests/forward_udp.rs` | yes | yes |
| `benches/shred_throughput` | no | yes |

---

## Binary

```bash
cargo run -- --bind=127.0.0.1:8001
```

Without `--bind=` it uses `127.0.0.1:8001`. The process:

1. Parses the addr (no I/O).
2. Builds the demo tree.
3. Creates `Pipeline::with_defaults` (FEC 2+1, shard 64).
4. Prints self, metrics (zeros), and the documentary flow.
5. Exits. It does **not** call `recv_into`.

A real loop would: `tokio_uring::start`, bind, `acquire`/`recv_into`/`try_send(SlotId)`/`ingest_slot`/`dest_addrs`/`forward_slot`/`release`. That is intentionally not in `main`.

---

## Tests

Convention: TDD. Unit tests live in `mod tests` in the same file; integration tests under `tests/`.

| Location | Coverage |
| --- | --- |
| `src/arena.rs` | Exhaustion, reuse, stale `SlotId`, `set_len`. |
| `src/shred.rs` | Truncation, bad type, round-trip, zero-copy, signatures. |
| `src/fec/reed_solomon.rs` | Encode/decode, erasures, bad config (`simd`). |
| `src/ingress/uring_udp.rs` | Linux recv/send loopback (`uring`). |
| `src/turbine/tree.rs` | Fanout 2/3, ties, empty cluster. |
| `src/metrics.rs` | Counters. |
| `tests/pipeline_ingest.rs` | Parse + FEC + dest_addrs + metrics + leader (`simd`). |
| `tests/forward_udp.rs` | UDP forward to children (`uring` + `simd`). |

```bash
cargo test
cargo test --no-default-features          # no FEC, no uring
```

Production modules: no `unwrap`/`expect`. Tests may use them.

---

## Benches

`benches/shred_throughput.rs` (Criterion 0.5, sample 20, 3 s per bench, `simd` feature):

| Bench | Measures |
| --- | --- |
| `parse/parse_data_shred` | `shred::parse` on an 88 B data shred. |
| `fec/encode_2_1_64` | Encode 2+1, 64 B shards. |
| `fec/decode_1_erasure` | Recover 1 data shard. |
| `arena/acquire_copy_release` | Acquire, copy 1228 B, release. |

Reference numbers (phase 8 close, Intel Core Ultra 9 275HX, Linux 6.18.7, 2026-08-15):

| Bench | Median | Criterion throughput |
| --- | --- | --- |
| parse data shred | 9.93 ns | 8.25 GiB/s |
| FEC encode 2+1 (64 B) | 31.8 ns | 5.63 GiB/s |
| FEC decode 1 erasure | 89.4 µs | 2.05 MiB/s |
| arena acquire/copy/release | 8.01 ns | 142.9 GiB/s |

This is **marked bytes / CPU time**, not the wire. Decode is far slower than encode at this shard size. The arena bench is an L1 memcpy. The live table is at the end of [`FASES.md`](FASES.md).

```bash
cargo bench --bench shred_throughput
```

---

## Project conventions

- **Single crate**, no workspace.
- Errors via `thiserror` only. No `anyhow`. `Error` is `Copy`.
- Hot path: no per-packet heap. Payloads are slices over the arena.
- `unsafe` is scoped and marked `SAFETY:` (packed headers, uring `IoBuf`, FEC scratch).
- Parse/validation hot path: `#[inline(always)]`.
- Concurrency: atomics and `crossbeam-channel`, not `Mutex`/`RwLock` on the shred path.
- `///` docs on **every** function.

---

## Phase plan

All phases are closed. Summary:

| Phase | Learning goal |
| --- | --- |
| 0 | Plan (`FASES.md`) and rules. |
| 1 | Crate + `Error`. |
| 2 | Slot arena. |
| 3 | Packed zero-copy parse. |
| 4 | SIMD Reed-Solomon. |
| 5 | UDP `io_uring`. |
| 6 | Turbine tree. |
| 7 | Logical pipeline. |
| 8 | UDP forward + benches. |
| extra | Signatures, metrics, features. |

The historical process (authorize a phase → TDD → implement only that scope) is documented in [`FASES.md`](FASES.md).

---

## Repository layout

```text
mini-solana-turbine/
├── Cargo.toml
├── FASES.md
├── README.md
├── README.en.md
├── src/
│   ├── lib.rs
│   ├── main.rs
│   ├── error.rs
│   ├── arena.rs
│   ├── pipeline.rs          # simd feature
│   ├── metrics.rs
│   ├── shred.rs
│   ├── ingress/
│   │   ├── mod.rs           # parse_addr, RecvDatagram
│   │   ├── datagram.rs
│   │   └── uring_udp.rs     # uring feature
│   ├── fec/
│   │   ├── mod.rs
│   │   └── reed_solomon.rs  # simd feature
│   └── turbine/
│       ├── mod.rs
│       └── tree.rs
├── tests/
│   ├── pipeline_ingest.rs
│   └── forward_udp.rs
└── benches/
    └── shred_throughput.rs
```

---

## Public API (summary)

Re-exported from `mini_solana_turbine`:

```text
PacketArena, SlotId, PACKET_SIZE
Error
parse_addr, RecvDatagram
UdpIngress                          # uring feature
FecEngine, DEFAULT_SHARD_BYTES      # simd feature
slot_queue, ForwardPlan, IngestResult, Pipeline   # simd feature
Metrics, MetricsSnapshot
Shred, DataShred, CodeShred, ShredHeader
ShredPublicKey, ShredSecretKey, SIGNATURE_BYTES
Node, NodeId, Stake, TurbineTree
```

Minimal loop (sketch; not what `main` runs):

```rust
// inside tokio_uring::start
let ingress = UdpIngress::bind("127.0.0.1:8001").await?;
let mut arena = PacketArena::<1024>::new();
let mut pipe = Pipeline::with_defaults(tree, self_id)?;
let datagram = ingress.recv_into(&mut arena).await?;
let result = pipe.ingest_slot(&arena, datagram.slot)?;
let mut dests = [ingress.local_addr()?; 8];
let n = pipe.dest_addrs(&result.forward(), &mut dests)?;
ingress.forward_slot(&arena, datagram.slot, &dests[..n]).await?;
arena.release(datagram.slot)?;
```

---

## Further docs

- Plan, scope, and bench numbers: [`FASES.md`](FASES.md) (Spanish)
- Rust API docs: `cargo doc --open`
- Spanish README: [README.md](README.md)
