# Plan de fases — mini-solana-turbine

Proyecto de aprendizaje: motor de ingestión y propagación de shreds al estilo Turbine (Solana).
Un solo crate. Cada fase requiere autorización explícita antes de empezar. Al cerrar una fase se informa qué se aplicó, el alcance real y se pide permiso para la siguiente.

## Decisiones cerradas (aplican a todas las fases)

| Tema | Decisión |
| --- | --- |
| Errores | Solo `thiserror`. No `anyhow`. |
| Documentación | Documentar **todas** las funciones (`///`: propósito, inputs, returns). Proyecto de aprendizaje. |
| `unsafe` | Permitido si está acotado, justificado y marcado con comentario `SAFETY:`. |
| Layout | Un crate, tree de módulos de `.cursorrules`. No workspace. |
| Hot path | Cero allocs de heap. Payloads = slices sobre **arenas estáticas** (tamaño fijo). No `Bytes` ni `Vec<u8>` para paquetes. |
| Tests | TDD: tests en el mismo archivo (`mod tests`) o en `tests/` **antes** de la implementación. |
| Producción | Sin `unwrap()` / `expect()` en módulos de producción. |
| Target | Linux x86_64, kernel 6.8+. |

`bytes` queda fuera del crate: el plan original lo listaba, pero choca con la política de arenas/slices.

---

## Fase 0 — Plan y scaffolding de repo

**Estado:** cerrada.

**Alcance**

- `FASES.md` (este archivo).
- `.gitignore`.
- Alinear `.cursorrules` y `rust.cursorrules` con las decisiones de la tabla.

**Fuera de alcance:** `Cargo.toml`, `src/`, benches.

**Criterio de cierre:** archivos de plan y gitignore en el repo; reglas sin contradicciones.

---

## Fase 1 — Crate, módulos vacíos y errores

**Estado:** cerrada (`cargo test` 8/8, `cargo clippy -D warnings` limpio).

**Objetivo de aprendizaje:** crate binario+librería, módulos públicos, errores zero-cost.

**Alcance**

- `Cargo.toml` (edition 2021+, `thiserror`; aún sin `tokio-uring` / `reed-solomon-simd` / `criterion`).
- `src/lib.rs`, `src/main.rs` (main mínimo que no hace I/O).
- Stubs documentados: `src/shred.rs`, `src/ingress/{mod,uring_udp}.rs`, `src/fec/{mod,reed_solomon}.rs`, `src/turbine/{mod,tree}.rs`.
- `src/error.rs`: enum `Error` con `thiserror` (variantes placeholder estables para fases siguientes).
- Test de humo: el crate compila y `Error` implementa `std::error::Error`.

**Fuera de alcance:** parseo de shreds, sockets, FEC, árbol Turbine, arenas reales.

**Criterio de cierre:** `cargo test` verde; funciones de stubs documentadas.

---

## Fase 2 — Arenas estáticas y buffers de paquete

**Estado:** cerrada (`cargo test` 18/18, `cargo clippy -D warnings` limpio).

**Objetivo de aprendizaje:** memoria de tamaño fijo, proyección a slices, cero heap en el camino de un paquete.

**Alcance**

- `src/arena.rs` (o similar): pool/arena de slots fijos (`PACKET_SIZE` alineado a MTU/shred Solana-like, p.ej. 1228 bytes útiles).
- API: adquirir slot → `&mut [u8]`, devolver slot, sin `Vec` en el hot path.
- Newtype para índice de slot (no `usize` crudo en APIs públicas).
- Tests primero: exhaustión, reuso, no overflow de slot, proyección de longitud real vs capacidad.

**`unsafe`:** solo si hace falta para inicializar arrays/MaybeUninit; bloques mínimos + `SAFETY:`.

**Fuera de alcance:** UDP, parseo de cabeceras.

**Criterio de cierre:** tests de arena verdes; ningún alloc en la API de acquire/release del slot.

---

## Fase 3 — Wire format y parseo zero-copy de shreds

**Estado:** cerrada (`cargo test` 30/30, `cargo clippy -D warnings` limpio).

**Objetivo de aprendizaje:** `#[repr(C)]` / packed, validación, proyección de bytes a structs sin serializar.

**Alcance**

- `ShredHeader`, `DataShred`, `CodeShred` en `src/shred.rs`.
- Parseo desde `&[u8]` del slot de arena → referencias / vistas, sin copiar payload.
- Validación: tamaños, flags data vs code, índices FEC.
- `#[inline(always)]` en parseo y validación de header.
- Tests TDD: shreds mínimos válidos, truncados, tipo incorrecto, round-trip de campos.

**`unsafe`:** `from_raw_parts` / cast de header solo tras chequear longitud y alineación; cada sitio con `SAFETY:`.

**Fuera de alcance:** reconstrucción FEC, red.

**Criterio de cierre:** tests de parseo/rechazo verdes; payloads siguen siendo slices sobre el slot.

---

## Fase 4 — FEC Reed-Solomon (SIMD)

**Estado:** cerrada (`cargo test` 36/36, `cargo clippy -D warnings` limpio).

**Objetivo de aprendizaje:** erasure coding, reconstruir data shreds a partir de un subconjunto + code shreds.

**Alcance**

- Dependencia `reed-solomon-simd`.
- `src/fec/reed_solomon.rs`: encode (data → code) y decode (erasure recovery).
- Buffers de trabajo reutilizados (no `Vec` por shred en el hot path; si la crate SIMD exige slices, salen de la arena o de scratch estático).
- Tests TDD: round-trip sin pérdidas; 1..N erasures dentro de capacidad; fallo si faltan demasiados.

**Fuera de alcance:** sockets, fanout.

**Criterio de cierre:** tests de reconstrucción verdes; errores vía `Error` / `FecError`.

---

## Fase 5 — Ingress UDP con io_uring

**Estado:** cerrada (`cargo test` 37/37, `cargo clippy -D warnings` limpio).

**Objetivo de aprendizaje:** recv UDP de baja latencia, llenar slots de arena desde el kernel sin copias extra.

**Alcance**

- Dependencias `tokio` / `tokio-uring` según lo que exija el recv.
- `src/ingress/uring_udp.rs`: bind, recv en slots de la arena, hand-off de índices de slot (no payloads clonados).
- Loop documentado; errores de socket mapeados a `thiserror`.
- Tests: loopback local (127.0.0.1) enviando un datagrama y comprobando bytes en el slot.

**Restricción:** Linux; si un test no puede abrir uring, se documenta y se marca `#[cfg(target_os = "linux")]`.

**Fuera de alcance:** árbol de peers, reconstrucción en el mismo PR mental.

**Criterio de cierre:** test de recv loopback verde en Linux; hot path sin `Vec` para el payload.

---

## Fase 6 — Árbol Turbine (stake-weighted fanout)

**Estado:** cerrada (`cargo test` 44/44, `cargo clippy -D warnings` limpio).

**Objetivo de aprendizaje:** quién reenvía a quién según stake; fanout fijo.

**Alcance**

- `src/turbine/tree.rs`: nodos (identity + stake + addr), ordenación por stake, cálculo de hijos dado `fanout`.
- Newtypes: `NodeId`, `Stake`.
- Tests TDD: árbol determinista, raíz, hojas, fanout 2 y 3, empates de stake (regla de desempate documentada).

**Fuera de alcance:** send UDP real (se puede exponer `children_of` puro).

**Criterio de cierre:** tests del árbol verdes; sin locks (`Mutex`/`RwLock`).

---

## Fase 7 — Pipeline: ingest → parse → FEC → fanout (lógico)

**Objetivo de aprendizaje:** unir módulos sin meter heap en el camino del shred.

**Alcance**

- API de pipeline en `lib.rs` o `src/pipeline.rs`: slot recibido → parse → acumular set FEC → reconstruir si se puede → calcular destinos Turbine.
- Cola lock-free (`crossbeam-channel`) o índices atómicos entre ingress y pipeline si hace falta un worker.
- `main.rs`: argumentos mínimos (bind addr, peers de ejemplo) y bucle.
- Test de integración en `tests/`: shreds sintéticos en memoria (sin red) recorren parse + FEC + tree.

**Fuera de alcance:** el envío UDP real a peers (puede quedar como stub `ForwardPlan`).

**Criterio de cierre:** test de integración verde; `main` arranca y documenta el flujo.

---

## Fase 8 — Forward UDP y benches

**Objetivo de aprendizaje:** medir, no “optimizar a ciegas”.

**Alcance**

- Envío UDP a hijos del árbol (reutilizando bytes del slot).
- `benches/shred_throughput.rs` con `criterion`: parseo, FEC, throughput de arena.
- Números de referencia escritos al cerrar la fase (comentario o sección al final de este archivo).

**Fuera de alcance:** cluster multi-máquina, gossip, firmas Ed25519 de shreds reales de mainnet (opcional en extra).

**Criterio de cierre:** `cargo bench` corre; `cargo test` sigue verde.

---

## Extra opcional (solo si se autoriza después)

- Verificación de firma de shred (compatibilidad educativa, no 100% Solana protocol).
- Métricas atómicas (`AtomicU64`: recibidos, reconstruidos, dropped).
- Feature flags en `Cargo.toml` (`uring`, `simd`).

---

## Cómo se trabaja cada fase

1. Pedir autorización (esta fase no empieza sin un “sí”).
2. Tests primero (TDD).
3. Implementar el alcance, nada más.
4. Documentar todas las funciones nuevas.
5. `cargo test` (y clippy si ya hay crate).
6. Informe de cierre: qué se aplicó, archivos tocados, qué quedó fuera, pedir autorización de la siguiente.

## Mapa de fases → archivos

| Fase | Archivos principales |
| --- | --- |
| 0 | `FASES.md`, `.gitignore`, `.cursorrules`, `rust.cursorrules` |
| 1 | `Cargo.toml`, `src/**` stubs, `src/error.rs` |
| 2 | `src/arena.rs`, tests en el mismo archivo |
| 3 | `src/shred.rs` |
| 4 | `src/fec/reed_solomon.rs` |
| 5 | `src/ingress/uring_udp.rs` |
| 6 | `src/turbine/tree.rs` |
| 7 | `src/pipeline.rs`, `src/main.rs`, `tests/` |
| 8 | `benches/shred_throughput.rs`, send path |
