# mini-solana-turbine

[English](README.en.md)

Motor **educativo** de ingestión y propagación de shreds al estilo [Turbine](https://docs.solana.com/cluster/turbine-block-propagation) (Solana), escrito en Rust. Recibe datagramas UDP en slots de una arena de tamaño fijo, los parsea sin copiar el payload, reconstruye shards con Reed-Solomon SIMD, calcula a quién reenviar según stake y (si el feature `uring` está activo) reenvía **los mismos bytes del slot**.

No es un validador Solana. No habla gossip, no verifica shreds de mainnet y no es compatible con el wire format de producción. Sirve para aprender el camino caliente: memoria fija, `io_uring`, FEC y fanout.

**Licencia:** MIT OR Apache-2.0 · **Crate:** `0.1.0` · `publish = false` · edición 2021.

---

## Índice

1. [Qué hace y qué no](#qué-hace-y-qué-no)
2. [Requisitos](#requisitos)
3. [Inicio rápido](#inicio-rápido)
4. [Cómo fluye un shred](#cómo-fluye-un-shred)
5. [Módulos](#módulos)
6. [Wire format (educativo)](#wire-format-educativo)
7. [Arena de paquetes](#arena-de-paquetes)
8. [FEC Reed-Solomon](#fec-reed-solomon)
9. [Árbol Turbine](#árbol-turbine)
10. [Ingress y forward UDP](#ingress-y-forward-udp)
11. [Firmas Ed25519](#firmas-ed25519)
12. [Métricas](#métricas)
13. [Feature flags](#feature-flags)
14. [Binario](#binario)
15. [Tests](#tests)
16. [Benches](#benches)
17. [Convenciones del proyecto](#convenciones-del-proyecto)
18. [Plan de fases](#plan-de-fases)
19. [Árbol del repositorio](#árbol-del-repositorio)
20. [API pública (resumen)](#api-pública-resumen)

---

## Qué hace y qué no

**Hace**

- Recibir UDP con `io_uring` (`tokio-uring`) hacia slots de `PacketArena`.
- Parsear shreds data (`0xA5`) y code (`0x5A`) zero-copy desde el slot.
- Reconstruir data shards faltantes con Reed-Solomon (`k` originales + `n` recovery).
- Ordenar un cluster por stake y devolver los hijos de fanout (árbol k-ario).
- Reenviar el payload del slot a esos hijos sin `Vec` ni clone.
- Firmar y verificar un prefijo Ed25519 de 64 bytes (esquema educativo).
- Contar `received` / `reconstructed` / `dropped` con `AtomicU64`.

**No hace**

- Cluster multi-máquina, discovery ni gossip.
- Layout 100 % Solana: no hay `ShredVariant`, merkle root, ni firmas de líderes de mainnet.
- Replay de ledger, TVU completo, ni consenso.
- Optimización “a ciegas”: los benches miden CPU, no Gbps de red.

El detalle de alcance por etapa está en [`FASES.md`](FASES.md).

---

## Requisitos

| Requisito | Valor |
| --- | --- |
| SO | Linux x86_64 |
| Kernel | 6.8+ (tests de `io_uring`) |
| Rust | edición 2021 (`rustc` reciente) |
| CPU | x86_64; AVX2 ayuda al FEC SIMD |

Sin Linux, el crate de shreds/arena/turbine/firmas sigue compilando, pero `UdpIngress` y los tests de loopback UDP están bajo `#[cfg(target_os = "linux")]`.

---

## Inicio rápido

```bash
# librería + tests (features por defecto: uring + simd)
cargo test

# clippy estricto (como en el cierre de cada fase)
cargo clippy --all-targets -- -D warnings

# documenta el flujo; no abre el socket uring
cargo run -- --bind=127.0.0.1:8001

# benches (necesita feature simd; tarda ~30 s)
cargo bench --bench shred_throughput
```

Otras combinaciones:

```bash
cargo test --no-default-features
cargo test --no-default-features --features simd
cargo test --no-default-features --features uring
```

El binario `mini-solana-turbine` exige `simd` (`required-features`).

---

## Cómo fluye un shred

```text
UDP datagrama
    │
    ▼
PacketArena::acquire          ─── un slot fijo (1228 B), sin heap
    │
    ▼
UdpIngress::recv_into         ─── el kernel escribe en el slot (io_uring)
    │
    ▼
slot_queue (crossbeam)        ─── se pasa SlotId, no el payload
    │
    ▼
Pipeline::ingest_slot
    ├─ [opcional] parse_signed  (Ed25519 si hay líder)
    ├─ shred::parse             (headers packed + payload prestado)
    ├─ scratch FEC              (copia el shard al set actual)
    ├─ try reconstruct          (si hay ≥ k shards)
    └─ ForwardPlan              (hijos Turbine de este nodo)
    │
    ▼
Pipeline::dest_addrs          ─── NodeId → SocketAddr (array en stack)
    │
    ▼
UdpIngress::forward_slot      ─── send_to con IoBuf sobre el mismo slot
    │
    ▼
PacketArena::release
```

`main` **no** entra en ese loop: valida `--bind`, construye un cluster de demo (4 nodos, fanout 2) y escribe el flujo en stdout. Abrir uring bloquearía el proceso de documentación.

---

## Módulos

| Módulo | Feature | Rol |
| --- | --- | --- |
| `arena` | siempre | Pool de slots `PACKET_SIZE = 1228`. |
| `shred` | siempre | Layout packed, parse/encode, firma educativa. |
| `error` | siempre | `thiserror`, tipo `Copy`, sin `String`. |
| `metrics` | siempre | `AtomicU64`: received, reconstructed, dropped. |
| `ingress` | `parse_addr` siempre; `UdpIngress` con `uring` | Recv/send UDP a slots. |
| `fec` | `simd` | `FecEngine` + `reed-solomon-simd`. |
| `pipeline` | `simd` | Une parse → FEC → `ForwardPlan`. |
| `turbine` | siempre | Árbol stake-weighted. |

Cada función pública (y la mayoría de las privadas) lleva doc `///` con Purpose, Inputs y Returns: es un proyecto de aprendizaje.

---

## Wire format (educativo)

Los headers son `#[repr(C, packed)]`. Los campos se leen **por valor** (nunca `&header.slot` sobre un packed).

### Paquete sin firmar

| Offset | Tamaño | Campo |
| --- | --- | --- |
| 0 | 8 | `slot` (u64 LE) |
| 8 | 4 | `fec_set_index` |
| 12 | 4 | `index` |
| 16 | 2 | `version` |
| 18 | 1 | `shred_type`: `0xA5` data, `0x5A` code |
| 19 | 1 | `reserved` |
| 20 | 4 ó 6 | subheader data (`parent_offset`, `flags`, `reserved`) o code (`num_data`, `num_code`, `position`) |
| resto | | payload (prestado del slot, no copiado) |

- Data: overhead **24 B**. Code: overhead **26 B**.
- `index >= fec_set_index`. En code, `position < num_code` y conteos ≠ 0.

### Paquete firmado

```text
[ firma Ed25519 64 B ][ body = layout de arriba ]
```

Se firma el **body** (todo lo que sigue a los 64 bytes), no el paquete entero. Igual que Solana pone la firma delante y firma el resto; **no** se firma un merkle root ni un shred de mainnet.

Funciones: `encode_signed_data` / `encode_signed_code`, `attach_signature`, `verify_signed`, `parse_signed`.

`parse` (sin prefijo) sigue existiendo: es el contrato de las fases 3–8.

---

## Arena de paquetes

`PacketArena<SLOTS>` reserva **un** `Box<[u8]>` en `new` (`SLOTS * 1228` bytes). Después:

- `acquire` → `SlotId` (índice + generación anti use-after-release).
- `slot` / `slot_mut` → `&[u8]` / `&mut [u8]` de longitud comprometida.
- `set_len` fija cuántos bytes son el datagrama (≤ 1228).
- `release` vuelve el slot al free-list.

Cero `Vec`/`Bytes` por paquete. `SlotId` stale o fuera de rango → `Error::ArenaSlotOutOfRange` / `ArenaExhausted`.

`DEFAULT_SLOT_COUNT = 1024` (~1,2 MiB).

El send path construye un `IoBuf` con el puntero del slot; el kernel lee esos bytes. El slot debe seguir ocupado hasta que termine el await.

---

## FEC Reed-Solomon

`FecEngine` (feature `simd`) envuelve encoder y decoder de `reed-solomon-simd` con workspace reutilizado.

- Default del pipeline: **k = 2** data, **n = 1** recovery, **`DEFAULT_SHARD_BYTES = 64`** (par, múltiplo de 64 para SIMD).
- `encode(&[&[u8]; k], &mut [&mut [u8]; n])` escribe paridad en buffers del caller.
- `decode` acepta `Option<&[u8]>` por shard (`None` = erasure) y escribe originales restaurados.
- Demasiadas erasures → `FecTooManyErasures`. Config inválida → `FecInconsistent`.

El pipeline copia cada payload de shred al scratch del set (`fec_set_index`). Cuando hay ≥ `k` shards y faltan data, reconstruye y marca `orig_present`.

Límite: `MAX_SHARDS = 16` (arrays en stack al reconstruir).

---

## Árbol Turbine

`turbine::tree::build(&[Node], fanout)`:

1. Copia los nodos.
2. Ordena por **stake descendente**; empate → `NodeId` **ascendente**.
3. Trata el vector como heap k-ario: hijos del índice `i` con fanout `f` son `f*i+1 .. f*i+f`.

`DEFAULT_FANOUT = 2`. `Node` = `NodeId` + `Stake` + `SocketAddr`. Sin `Mutex`.

`children_of(id, &mut [NodeId])` escribe en un buffer del caller (el `ForwardPlan` cabe `MAX_FORWARD = 8` destinos).

Demo de `main` (4 nodos, fanout 2, raíz = id 1 por más stake):

```text
        1 (100)
       / \
      2    3
     (50) (40)
      |
      4 (10)
```

---

## Ingress y forward UDP

Requiere feature **`uring`** y runtime `tokio_uring::start`.

| Método | Qué hace |
| --- | --- |
| `UdpIngress::bind` / `bind_addr` | Bind UDP io_uring. |
| `recv_into(&mut arena)` | Acquire + `recv_from` al slot. |
| `send_slot(&arena, slot, dest)` | Envía `arena.slot(slot)` sin clone. |
| `forward_slot(&arena, slot, &[SocketAddr])` | Un send por destino. |

`parse_addr("127.0.0.1:0")` **no** necesita `uring`.

Errores: `IngressBind`, `IngressRecv`, `IngressSend`.

El test `tests/forward_udp.rs` hace loopback: ingest de un shred en arena → destinos del árbol → dos sockets std reciben los **mismos** bytes del slot.

---

## Firmas Ed25519

Crate `ed25519-dalek` (siempre, no es un feature).

- `ShredSecretKey` / `ShredPublicKey`: 32 bytes. `Debug` de la secreta no imprime la semilla.
- `Pipeline::require_leader(pk)` hace que `ingest_*` exija `sig \|\| body`.
- `clear_leader()` vuelve al parseo sin firma.
- Errores: `ShredBadSignature`, `ShredInvalidKey`, o `ShredTruncated` si hay menos de 64 bytes.

No uses estas firmas contra un cluster real de Solana.

---

## Métricas

`Pipeline::metrics() -> &Metrics` (también usable suelto):

| Contador | Cuándo sube |
| --- | --- |
| `received` | Cada `ingest_bytes` / `ingest_slot` (ok o error). |
| `reconstructed` | Suma de data shards que el FEC acaba de restaurar. |
| `dropped` | Ingest que devolvió `Err` (parse, firma, FEC, …). |

Cargas y `fetch_add` con `Ordering::Relaxed`. `snapshot()` copia los tres valores.

---

## Feature flags

```toml
[features]
default = ["uring", "simd"]
uring = ["dep:tokio-uring"]   # UdpIngress
simd  = ["dep:reed-solomon-simd"]  # FecEngine + Pipeline
```

| Qué | `uring` | `simd` |
| --- | --- | --- |
| Arena, shred, turbine, métricas, firmas, `parse_addr` | no | no |
| `UdpIngress` | sí | no |
| `FecEngine`, `Pipeline`, `slot_queue` | no | sí |
| Binario | no | **sí** (`required-features`) |
| `tests/pipeline_ingest.rs` | no | sí |
| `tests/forward_udp.rs` | sí | sí |
| `benches/shred_throughput` | no | sí |

---

## Binario

```bash
cargo run -- --bind=127.0.0.1:8001
```

Sin `--bind=` usa `127.0.0.1:8001`. El proceso:

1. Parsea el addr (sin I/O).
2. Construye el árbol de demo.
3. Crea `Pipeline::with_defaults` (FEC 2+1, shard 64).
4. Imprime self, métricas (ceros) y el flujo documental.
5. Sale. **No** llama a `recv_into`.

Para un loop real habría que: `tokio_uring::start`, bind, `acquire`/`recv_into`/`try_send(SlotId)`/`ingest_slot`/`dest_addrs`/`forward_slot`/`release`. Eso no está en `main` a propósito.

---

## Tests

Convención: TDD. Tests unitarios en `mod tests` del mismo archivo; integración en `tests/`.

| Sitio | Contenido |
| --- | --- |
| `src/arena.rs` | Exhaustión, reuso, stale `SlotId`, `set_len`. |
| `src/shred.rs` | Truncado, tipo inválido, round-trip, zero-copy, firmas. |
| `src/fec/reed_solomon.rs` | Encode/decode, erasures, config inválida (`simd`). |
| `src/ingress/uring_udp.rs` | Loopback recv/send Linux (`uring`). |
| `src/turbine/tree.rs` | Fanout 2/3, empates, cluster vacío. |
| `src/metrics.rs` | Contadores. |
| `tests/pipeline_ingest.rs` | Parse + FEC + dest_addrs + métricas + líder (`simd`). |
| `tests/forward_udp.rs` | Forward UDP a hijos (`uring` + `simd`). |

```bash
cargo test
cargo test --no-default-features          # sin FEC ni uring
```

Módulos de producción: sin `unwrap`/`expect`. Los tests sí pueden usarlos.

---

## Benches

`benches/shred_throughput.rs` (Criterion 0.5, sample 20, 3 s por bench, feature `simd`):

| Bench | Qué mide |
| --- | --- |
| `parse/parse_data_shred` | `shred::parse` sobre un data shred de 88 B. |
| `fec/encode_2_1_64` | Encode 2+1, shards de 64 B. |
| `fec/decode_1_erasure` | Recupera 1 data shard. |
| `arena/acquire_copy_release` | Acquire, copiar 1228 B, release. |

Números de referencia (cierre fase 8, Intel Core Ultra 9 275HX, Linux 6.18.7, 2026-08-15):

| Bench | Mediana | Throughput Criterion |
| --- | --- | --- |
| parse data shred | 9,93 ns | 8,25 GiB/s |
| FEC encode 2+1 (64 B) | 31,8 ns | 5,63 GiB/s |
| FEC decode 1 erasure | 89,4 µs | 2,05 MiB/s |
| arena acquire/copy/release | 8,01 ns | 142,9 GiB/s |

Eso es **bytes marcados / tiempo de CPU**, no el cable. El decode es mucho más lento que el encode con este tamaño de shard. La arena mide memcpy en L1. La tabla viva está al final de [`FASES.md`](FASES.md).

```bash
cargo bench --bench shred_throughput
```

---

## Convenciones del proyecto

- **Un crate**, no workspace.
- Errores solo con `thiserror`. No `anyhow`. `Error` es `Copy`.
- Hot path: sin heap por paquete. Payloads = slices sobre la arena.
- `unsafe` acotado + comentario `SAFETY:` (headers packed, `IoBuf` de uring, scratch FEC).
- Hot path de parseo/validación: `#[inline(always)]`.
- Concurrencia: atomics y `crossbeam-channel`, no `Mutex`/`RwLock` en el camino del shred.
- Docs `///` en **todas** las funciones.

---

## Plan de fases

Todas cerradas. Resumen:

| Fase | Aprendizaje |
| --- | --- |
| 0 | Plan (`FASES.md`) y reglas. |
| 1 | Crate + `Error`. |
| 2 | Arena de slots. |
| 3 | Parseo packed zero-copy. |
| 4 | Reed-Solomon SIMD. |
| 5 | UDP `io_uring`. |
| 6 | Árbol Turbine. |
| 7 | Pipeline lógico. |
| 8 | Forward UDP + benches. |
| extra | Firmas, métricas, features. |

El procedimiento histórico (autorizar fase → TDD → implementar solo el alcance) está en [`FASES.md`](FASES.md).

---

## Árbol del repositorio

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
│   ├── pipeline.rs          # feature simd
│   ├── metrics.rs
│   ├── shred.rs
│   ├── ingress/
│   │   ├── mod.rs           # parse_addr, RecvDatagram
│   │   ├── datagram.rs
│   │   └── uring_udp.rs     # feature uring
│   ├── fec/
│   │   ├── mod.rs
│   │   └── reed_solomon.rs  # feature simd
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

## API pública (resumen)

Reexportada desde `mini_solana_turbine`:

```text
PacketArena, SlotId, PACKET_SIZE
Error
parse_addr, RecvDatagram
UdpIngress                          # feature uring
FecEngine, DEFAULT_SHARD_BYTES      # feature simd
slot_queue, ForwardPlan, IngestResult, Pipeline   # feature simd
Metrics, MetricsSnapshot
Shred, DataShred, CodeShred, ShredHeader
ShredPublicKey, ShredSecretKey, SIGNATURE_BYTES
Node, NodeId, Stake, TurbineTree
```

El loop mínimo (esqueleto, no está en `main`):

```rust
// dentro de tokio_uring::start
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

## Documentación extra

- Plan, alcance y números de bench: [`FASES.md`](FASES.md)
- Docs de código: `cargo doc --open`
- Traducción: [README.en.md](README.en.md)
