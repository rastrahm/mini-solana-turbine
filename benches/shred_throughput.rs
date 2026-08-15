//! Benches de referencia: parseo zero-copy, Reed-Solomon SIMD y arena de slots.
//!
//! Números de cierre: ver `FASES.md` (Fase 8). Config: sample 20, 3 s de medición.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use mini_solana_turbine::shred::{self, DataShredHeader, ShredHeader};
use mini_solana_turbine::{FecEngine, PacketArena, DEFAULT_SHARD_BYTES, PACKET_SIZE};
use std::time::Duration;

/// Payload de data shred de bench: un shard FEC de 64 B (cabe en el slot).
const SHRED_PAYLOAD: usize = DEFAULT_SHARD_BYTES;

/// Purpose: Config corta para que `cargo bench` termine en ~30 s, no minutos.
/// Inputs: none.
/// Returns: Criterion con sample 20 y 3 s de wall time por bench.
fn criterion_config() -> Criterion {
    Criterion::default()
        .sample_size(20)
        .measurement_time(Duration::from_secs(3))
}

/// Purpose: Prepara un data shred válido en un array de stack.
/// Inputs: `dest` — al menos `PACKET_SIZE`.
/// Returns: bytes escritos, o panics de setup si encode falla.
fn encode_sample_shred(dest: &mut [u8]) -> usize {
    let mut payload = [0u8; SHRED_PAYLOAD];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = i as u8;
    }
    match shred::encode_data(
        dest,
        ShredHeader::data(1, 0, 0, 1),
        DataShredHeader::new(1, 0),
        &payload,
    ) {
        Ok(n) => n,
        Err(_) => panic!("bench setup: encode_data"),
    }
}

/// Purpose: Throughput de [`shred::parse`] sobre un paquete ya en stack.
/// Inputs: `c` — harness de criterion.
/// Returns: none (registra el bench `parse_data_shred`).
fn bench_parse(c: &mut Criterion) {
    let mut pkt = [0u8; PACKET_SIZE];
    let n = encode_sample_shred(&mut pkt);
    let mut group = c.benchmark_group("parse");
    group.throughput(Throughput::Bytes(n as u64));
    group.bench_function("parse_data_shred", |b| {
        b.iter(|| {
            let shred = shred::parse(black_box(&pkt[..n]));
            black_box(shred)
        })
    });
    group.finish();
}

/// Purpose: Encode 2+1 y decode con 1 erasure (shard 64 B).
/// Inputs: `c` — harness.
/// Returns: none (grupos `fec/encode_2_1_64` y `fec/decode_1_erasure`).
fn bench_fec(c: &mut Criterion) {
    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    for (i, b) in d0.iter_mut().enumerate() {
        *b = i as u8;
    }
    for (i, b) in d1.iter_mut().enumerate() {
        *b = 0xA0u8.wrapping_add(i as u8);
    }
    let bytes_set = (DEFAULT_SHARD_BYTES * 3) as u64;

    let mut group = c.benchmark_group("fec");
    group.throughput(Throughput::Bytes(bytes_set));

    group.bench_function("encode_2_1_64", |b| {
        let mut engine = match FecEngine::new(2, 1, DEFAULT_SHARD_BYTES) {
            Ok(e) => e,
            Err(_) => panic!("bench setup: FecEngine"),
        };
        let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
        b.iter(|| {
            let orig = [d0.as_slice(), d1.as_slice()];
            let mut rec = [r0.as_mut_slice()];
            let res = engine.encode(black_box(&orig), black_box(&mut rec));
            black_box(res)
        })
    });

    let mut rec_fixed = [0u8; DEFAULT_SHARD_BYTES];
    {
        let mut engine = match FecEngine::new(2, 1, DEFAULT_SHARD_BYTES) {
            Ok(e) => e,
            Err(_) => panic!("bench setup: FecEngine encode"),
        };
        match engine.encode(&[&d0, &d1], &mut [&mut rec_fixed]) {
            Ok(()) => {}
            Err(_) => panic!("bench setup: fec encode"),
        }
    }

    group.bench_function("decode_1_erasure", |b| {
        let mut engine = match FecEngine::new(2, 1, DEFAULT_SHARD_BYTES) {
            Ok(e) => e,
            Err(_) => panic!("bench setup: FecEngine decode"),
        };
        let mut restored0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut restored1 = [0u8; DEFAULT_SHARD_BYTES];
        b.iter(|| {
            let original = [Some(d0.as_slice()), None];
            let recovery = [Some(rec_fixed.as_slice())];
            let mut dests = [restored0.as_mut_slice(), restored1.as_mut_slice()];
            let n = engine.decode(
                black_box(&original),
                black_box(&recovery),
                black_box(&mut dests),
            );
            black_box(n)
        })
    });
    group.finish();
}

/// Purpose: Acquire → copiar payload de slot → release (ancho de banda de arena).
/// Inputs: `c` — harness.
/// Returns: none (`arena/acquire_copy_release`).
fn bench_arena(c: &mut Criterion) {
    let mut src = [0u8; PACKET_SIZE];
    for (i, b) in src.iter_mut().enumerate() {
        *b = i as u8;
    }
    let mut group = c.benchmark_group("arena");
    group.throughput(Throughput::Bytes(PACKET_SIZE as u64));
    group.bench_function("acquire_copy_release", |b| {
        let mut arena = PacketArena::<64>::new();
        b.iter(|| {
            let slot = match arena.acquire() {
                Ok(s) => s,
                Err(_) => panic!("bench: arena exhausted"),
            };
            match arena.slot_mut(slot) {
                Ok(buf) => buf.copy_from_slice(black_box(&src)),
                Err(_) => panic!("bench: slot_mut"),
            }
            match arena.set_len(slot, PACKET_SIZE) {
                Ok(()) => {}
                Err(_) => panic!("bench: set_len"),
            }
            match arena.release(slot) {
                Ok(()) => {}
                Err(_) => panic!("bench: release"),
            }
        })
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = bench_parse, bench_fec, bench_arena
}
criterion_main!(benches);
