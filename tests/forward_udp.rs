//! Integración: ingest → dest_addrs → forward_slot (bytes del arena, sin clone).

use mini_solana_turbine::pipeline::MAX_FORWARD;
use mini_solana_turbine::shred::{self, DataShredHeader, ShredHeader};
use mini_solana_turbine::turbine::{self, Node, NodeId, Stake};
use mini_solana_turbine::{
    Error, FecEngine, PacketArena, Pipeline, UdpIngress, DEFAULT_SHARD_BYTES, PACKET_SIZE,
};
use std::net::UdpSocket;
use std::time::Duration;

/// Purpose: Rellena un shard con un patrón.
/// Inputs: `dest`, `tag`.
/// Returns: none.
fn fill(dest: &mut [u8], tag: u8) {
    for (i, b) in dest.iter_mut().enumerate() {
        *b = tag.wrapping_add(i as u8);
    }
}

/// Purpose: Loopback: el slot ingerido llega a los dos hijos por UDP.
/// Inputs: none.
/// Returns: panics si algún hijo no recibe los mismos bytes del arena.
#[cfg(target_os = "linux")]
#[test]
fn forward_slot_reaches_tree_children() {
    let result = tokio_uring::start(async { forward_children_body().await });
    result.expect("forward children");
}

/// Purpose: Cuerpo async: bind uring + dos recv std, árbol fanout 2, send del slot.
/// Inputs: none.
/// Returns: `Ok` si ambos datagramas coinciden con `arena.slot`.
#[cfg(target_os = "linux")]
async fn forward_children_body() -> Result<(), Error> {
    let child_a = UdpSocket::bind("127.0.0.1:0").map_err(|_| Error::IngressBind)?;
    let child_b = UdpSocket::bind("127.0.0.1:0").map_err(|_| Error::IngressBind)?;
    child_a
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| Error::IngressBind)?;
    child_b
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| Error::IngressBind)?;
    let addr_a = child_a.local_addr().map_err(|_| Error::IngressBind)?;
    let addr_b = child_b.local_addr().map_err(|_| Error::IngressBind)?;

    let ingress = UdpIngress::bind("127.0.0.1:0").await?;
    let self_addr = ingress.local_addr()?;

    let tree = turbine::tree::build(
        &[
            Node::new(NodeId::new(1), Stake::new(100), self_addr),
            Node::new(NodeId::new(2), Stake::new(50), addr_a),
            Node::new(NodeId::new(3), Stake::new(40), addr_b),
        ],
        2,
    )?;
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(1))?;

    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    fill(&mut d0, 11);
    fill(&mut d1, 12);
    let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
    FecEngine::new(2, 1, DEFAULT_SHARD_BYTES)?.encode(&[&d0, &d1], &mut [&mut r0])?;

    let mut arena = PacketArena::<2>::new();
    let slot = arena.acquire()?;
    let pkt_len = {
        let buf = arena.slot_mut(slot)?;
        shred::encode_data(
            buf,
            ShredHeader::data(1, 0, 0, 1),
            DataShredHeader::new(1, 0),
            &d0,
        )?
    };
    arena.set_len(slot, pkt_len)?;

    let result = pipe.ingest_slot(&arena, slot)?;
    let mut dests = [self_addr; MAX_FORWARD];
    let n = pipe.dest_addrs(&result.forward(), &mut dests)?;
    assert_eq!(n, 2);

    let sent = ingress.forward_slot(&arena, slot, &dests[..n]).await?;
    assert_eq!(sent, 2);

    let expected = arena.slot(slot)?;
    assert_recv_equals(&child_a, expected)?;
    assert_recv_equals(&child_b, expected)?;
    let _ = arena.release(slot);
    Ok(())
}

/// Purpose: Lee un datagrama y lo compara con el slice del slot.
/// Inputs: `sock` — hijo; `expected` — bytes del arena (no un `Vec` propio).
/// Returns: `Ok` o `IngressRecv` si timeout / mismatch.
fn assert_recv_equals(sock: &UdpSocket, expected: &[u8]) -> Result<(), Error> {
    let mut buf = [0u8; PACKET_SIZE];
    let (n, _) = sock.recv_from(&mut buf).map_err(|_| Error::IngressRecv)?;
    if &buf[..n] != expected {
        return Err(Error::IngressRecv);
    }
    Ok(())
}

/// Purpose: Evita warning de import no usado fuera de Linux (el test está gated).
/// Inputs: none.
/// Returns: panics si PacketArena no reserva.
#[cfg(not(target_os = "linux"))]
#[test]
fn forward_udp_skipped_off_linux() {
    let _ = PacketArena::<1>::new();
}
