//! Integración en memoria: parse + FEC + árbol, sin red.

use mini_solana_turbine::pipeline::MAX_FORWARD;
use mini_solana_turbine::shred::{
    self, CodeShredHeader, DataShredHeader, ShredHeader, ShredSecretKey,
};
use mini_solana_turbine::turbine::{self, Node, NodeId, Stake};
use mini_solana_turbine::{
    slot_queue, Error, FecEngine, PacketArena, Pipeline, DEFAULT_SHARD_BYTES, PACKET_SIZE,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Purpose: Addr local por id.
/// Inputs: `id`.
/// Returns: `127.0.0.1:9000+id`.
fn addr(id: u32) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000 + id as u16)
}

/// Purpose: Cluster de 4 nodos; el id 1 es raíz (más stake) y tiene 2 hijos.
/// Inputs: none.
/// Returns: árbol fanout 2.
fn sample_tree() -> Result<mini_solana_turbine::TurbineTree, Error> {
    turbine::tree::build(
        &[
            Node::new(NodeId::new(1), Stake::new(100), addr(1)),
            Node::new(NodeId::new(2), Stake::new(50), addr(2)),
            Node::new(NodeId::new(3), Stake::new(40), addr(3)),
            Node::new(NodeId::new(4), Stake::new(10), addr(4)),
        ],
        2,
    )
}

/// Purpose: Rellena un shard con un patrón.
/// Inputs: `dest`, `tag`.
/// Returns: none.
fn fill(dest: &mut [u8], tag: u8) {
    for (i, b) in dest.iter_mut().enumerate() {
        *b = tag.wrapping_add(i as u8);
    }
}

/// Purpose: Ingerir data0 + code reconstruye data1 y propone hijos de la raíz.
/// Inputs: none.
/// Returns: panics si no se reconstruye el shard 1 o el plan está vacío.
#[test]
fn reconstructs_missing_data_and_plans_forward() {
    let tree = sample_tree().expect("tree");
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(1)).expect("pipe");

    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d0, 1);
    fill(&mut d1, 2);
    let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
    let mut engine = FecEngine::new(2, 1, DEFAULT_SHARD_BYTES).expect("fec");
    engine
        .encode(&[&d0, &d1], &mut [&mut r0])
        .expect("encode fec");

    let mut pkt0 = [0u8; PACKET_SIZE];
    let n0 = shred::encode_data(
        &mut pkt0,
        ShredHeader::data(1, 0, 0, 1),
        DataShredHeader::new(1, 0),
        &d0,
    )
    .expect("enc d0");
    let mut pktc = [0u8; PACKET_SIZE];
    let nc = shred::encode_code(
        &mut pktc,
        ShredHeader::code(1, 0, 2, 1),
        CodeShredHeader::new(2, 1, 0),
        &r0,
    )
    .expect("enc c");

    let first = pipe.ingest_bytes(&pkt0[..n0]).expect("ingest d0");
    assert_eq!(first.reconstructed(), 0);
    assert_eq!(first.forward().dests(), &[NodeId::new(2), NodeId::new(3)]);
    assert_eq!(pipe.metrics().received(), 1);
    assert_eq!(pipe.metrics().dropped(), 0);
    assert_eq!(pipe.metrics().reconstructed(), 0);

    let second = pipe.ingest_bytes(&pktc[..nc]).expect("ingest code");
    assert_eq!(second.reconstructed(), 1);
    assert_eq!(pipe.original_shard(1).expect("restored"), &d1[..]);
    assert_eq!(pipe.metrics().received(), 2);
    assert_eq!(pipe.metrics().reconstructed(), 1);
    assert_eq!(pipe.metrics().dropped(), 0);
}

/// Purpose: Slot de arena + cola lock-free llegan al mismo ingest.
/// Inputs: none.
/// Returns: panics si el receiver no entrega el slot parseable.
#[test]
fn arena_slot_queue_feeds_pipeline() {
    let tree = sample_tree().expect("tree");
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(1)).expect("pipe");
    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d0, 9);
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d1, 8);
    let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
    FecEngine::new(2, 1, DEFAULT_SHARD_BYTES)
        .expect("fec")
        .encode(&[&d0, &d1], &mut [&mut r0])
        .expect("need both originals for a valid encode");

    let mut arena = PacketArena::<2>::new();
    let slot = arena.acquire().expect("slot");
    let n = {
        let buf = arena.slot_mut(slot).expect("mut");
        shred::encode_data(
            buf,
            ShredHeader::data(1, 0, 0, 1),
            DataShredHeader::new(1, 0),
            &d0,
        )
        .expect("enc")
    };
    arena.set_len(slot, n).expect("len");

    let (tx, rx) = slot_queue(4);
    match tx.try_send(slot) {
        Ok(()) => {}
        Err(_) => panic!("queue"),
    }
    let got = match rx.try_recv() {
        Ok(id) => id,
        Err(_) => panic!("recv"),
    };
    let result = pipe.ingest_slot(&arena, got).expect("ingest slot");
    assert_eq!(result.reconstructed(), 0);
    assert!(!result.forward().is_empty());
}

/// Purpose: `dest_addrs` resuelve hijos de la raíz a `127.0.0.1:9002/9003`.
/// Inputs: none.
/// Returns: panics si los SocketAddr no coinciden con el cluster.
#[test]
fn dest_addrs_resolves_tree_children() {
    let tree = sample_tree().expect("tree");
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(1)).expect("pipe");
    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d0, 3);
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d1, 4);
    let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
    FecEngine::new(2, 1, DEFAULT_SHARD_BYTES)
        .expect("fec")
        .encode(&[&d0, &d1], &mut [&mut r0])
        .expect("encode");

    let mut pkt = [0u8; PACKET_SIZE];
    let n = shred::encode_data(
        &mut pkt,
        ShredHeader::data(1, 0, 0, 1),
        DataShredHeader::new(1, 0),
        &d0,
    )
    .expect("enc");
    let plan = pipe.ingest_bytes(&pkt[..n]).expect("ingest").forward();

    let mut addrs = [addr(0); MAX_FORWARD];
    let n_dest = pipe.dest_addrs(&plan, &mut addrs).expect("addrs");
    assert_eq!(n_dest, 2);
    assert_eq!(&addrs[..n_dest], &[addr(2), addr(3)]);
}

/// Purpose: Una hoja no tiene destinos UDP.
/// Inputs: none.
/// Returns: panics si `dest_addrs` no da 0.
#[test]
fn dest_addrs_leaf_is_empty() {
    let tree = sample_tree().expect("tree");
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(4)).expect("pipe");
    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d0, 5);
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d1, 6);
    let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
    FecEngine::new(2, 1, DEFAULT_SHARD_BYTES)
        .expect("fec")
        .encode(&[&d0, &d1], &mut [&mut r0])
        .expect("encode");

    let mut pkt = [0u8; PACKET_SIZE];
    let n = shred::encode_data(
        &mut pkt,
        ShredHeader::data(1, 0, 0, 1),
        DataShredHeader::new(1, 0),
        &d0,
    )
    .expect("enc");
    let plan = pipe.ingest_bytes(&pkt[..n]).expect("ingest").forward();
    let mut addrs = [addr(0); MAX_FORWARD];
    let n_dest = pipe.dest_addrs(&plan, &mut addrs).expect("addrs");
    assert_eq!(n_dest, 0);
}

/// Purpose: Un paquete basura incrementa received y dropped.
/// Inputs: none.
/// Returns: panics si dropped no es 1.
#[test]
fn ingest_error_counts_as_dropped() {
    let tree = sample_tree().expect("tree");
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(1)).expect("pipe");
    assert_eq!(pipe.ingest_bytes(&[]).err(), Some(Error::ShredTruncated));
    assert_eq!(pipe.metrics().received(), 1);
    assert_eq!(pipe.metrics().dropped(), 1);
    assert_eq!(pipe.metrics().reconstructed(), 0);
}

/// Purpose: Con líder, solo entra un data shred firmado.
/// Inputs: none.
/// Returns: panics si el unsigned se droppea o el firmado no pasa.
#[test]
fn ingest_signed_requires_leader_key() {
    let tree = sample_tree().expect("tree");
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(1)).expect("pipe");
    let sk = ShredSecretKey::from_bytes([11u8; 32]);
    pipe.require_leader(sk.public());

    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d0, 21);
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d1, 22);
    let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
    FecEngine::new(2, 1, DEFAULT_SHARD_BYTES)
        .expect("fec")
        .encode(&[&d0, &d1], &mut [&mut r0])
        .expect("encode");

    let mut unsigned = [0u8; PACKET_SIZE];
    let nu = shred::encode_data(
        &mut unsigned,
        ShredHeader::data(1, 0, 0, 1),
        DataShredHeader::new(1, 0),
        &d0,
    )
    .expect("enc unsigned");
    assert_eq!(
        pipe.ingest_bytes(&unsigned[..nu]),
        Err(Error::ShredBadSignature)
    );
    assert_eq!(pipe.metrics().dropped(), 1);

    let mut signed = [0u8; PACKET_SIZE];
    let ns = shred::encode_signed_data(
        &mut signed,
        &sk,
        ShredHeader::data(1, 0, 0, 1),
        DataShredHeader::new(1, 0),
        &d0,
    )
    .expect("enc signed");
    let ok = pipe.ingest_bytes(&signed[..ns]).expect("ingest signed");
    assert_eq!(ok.reconstructed(), 0);
}
