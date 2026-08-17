//! Sesión de debug: genera una petición UDP y la recorre hasta el fanout.
//!
//! Solo se compila en el binario con feature `uring`. No forma parte de la lib.

use mini_solana_turbine::pipeline::{Pipeline, MAX_FORWARD};
use mini_solana_turbine::shred::{self, DataShredHeader, ShredHeader};
use mini_solana_turbine::turbine::{self, Node, NodeId, Stake};
use mini_solana_turbine::{
    Error, FecEngine, PacketArena, UdpIngress, DEFAULT_SHARD_BYTES, PACKET_SIZE,
};
use std::io::{self, Write};
use std::net::UdpSocket;
use std::time::Duration;

/// Purpose: Rellena un shard con un patrón repetible (`tag + i`).
/// Inputs: `dest` — 64 B; `tag` — semilla.
/// Returns: none.
fn fill_shard(dest: &mut [u8], tag: u8) {
    for (i, b) in dest.iter_mut().enumerate() {
        *b = tag.wrapping_add(i as u8);
    }
}

/// Purpose: Escribe `n` bytes en hex separados por espacio (sin heap extra).
/// Inputs: `out` — stdout; `bytes` — paquete; `n` — cuántos prefijo.
/// Returns: `io::Result`.
fn write_hex_prefix(out: &mut impl Write, bytes: &[u8], n: usize) -> io::Result<()> {
    for (i, b) in bytes.iter().copied().take(n).enumerate() {
        if i > 0 {
            write!(out, " ")?;
        }
        write!(out, "{b:02x}")?;
    }
    Ok(())
}

/// Purpose: Bind std UDP con timeout de lectura (hijo del árbol / sonda).
/// Inputs: none (`127.0.0.1:0`).
/// Returns: socket o `IngressBind`.
fn bind_std_listener() -> Result<UdpSocket, Error> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|_| Error::IngressBind)?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| Error::IngressBind)?;
    Ok(sock)
}

/// Purpose: Recibe un datagrama y comprueba que coincide con `expected`.
/// Inputs: `sock` — hijo; `expected` — bytes del slot (sin clonarlo a un `Vec`).
/// Returns: bytes leídos, o `IngressRecv`.
fn recv_match(sock: &UdpSocket, expected: &[u8]) -> Result<usize, Error> {
    let mut buf = [0u8; PACKET_SIZE];
    let (n, _) = sock.recv_from(&mut buf).map_err(|_| Error::IngressRecv)?;
    if &buf[..n] != expected {
        return Err(Error::IngressRecv);
    }
    Ok(n)
}

/// Purpose: Codifica un data shred (set FEC 2+1, shard 0) sobre `dest`.
/// Inputs: `dest` — capacidad de slot; `d0` — payload de 64 B.
/// Returns: longitud del paquete.
fn encode_debug_shred(dest: &mut [u8], d0: &[u8]) -> Result<usize, Error> {
    shred::encode_data(
        dest,
        ShredHeader::data(1, 0, 0, 1),
        DataShredHeader::new(1, 0),
        d0,
    )
}

/// Purpose: Loopback documentado: cliente std → recv uring → ingest → forward.
/// Inputs: none (puertos efímeros).
/// Returns: `Ok` si hijos reciben el mismo payload; error de red/parse/FEC.
pub async fn run_session() -> Result<(), Error> {
    let mut out = io::stdout();
    let _ = writeln!(
        out,
        "=== debug-udp: origen de las peticiones ===\n\
         A) cliente std        → primer datagrama (petición de ingestión)\n\
         B) UdpIngress::forward_slot → reenvío a hijos Turbine (mismos bytes del slot)"
    );

    let child_a = bind_std_listener()?;
    let child_b = bind_std_listener()?;
    let addr_a = child_a.local_addr().map_err(|_| Error::IngressBind)?;
    let addr_b = child_b.local_addr().map_err(|_| Error::IngressBind)?;

    let ingress = UdpIngress::bind("127.0.0.1:0").await?;
    let ingress_addr = ingress.local_addr()?;
    let _ = writeln!(out, "[setup] ingress (uring) {ingress_addr}");
    let _ = writeln!(out, "[setup] hijo A (std)    {addr_a}");
    let _ = writeln!(out, "[setup] hijo B (std)    {addr_b}");

    let tree = turbine::tree::build(
        &[
            Node::new(NodeId::new(1), Stake::new(100), ingress_addr),
            Node::new(NodeId::new(2), Stake::new(50), addr_a),
            Node::new(NodeId::new(3), Stake::new(40), addr_b),
        ],
        2,
    )?;
    let mut pipe = Pipeline::with_defaults(tree, NodeId::new(1))?;

    let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
    let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
    fill_shard(&mut d0, 11);
    fill_shard(&mut d1, 12);
    let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
    FecEngine::new(2, 1, DEFAULT_SHARD_BYTES)?.encode(&[&d0, &d1], &mut [&mut r0])?;

    let mut pkt = [0u8; PACKET_SIZE];
    let pkt_len = encode_debug_shred(&mut pkt, &d0)?;
    let _ = writeln!(
        out,
        "\n[1] encode_data  slot=1 fec=0 index=0  body={pkt_len} B  (PAYLOAD no se clona luego)"
    );
    let _ = write!(out, "    header hex: ");
    let _ = write_hex_prefix(&mut out, &pkt[..pkt_len], 24.min(pkt_len));
    let _ = writeln!(out);

    let client = UdpSocket::bind("127.0.0.1:0").map_err(|_| Error::IngressBind)?;
    let client_addr = client.local_addr().map_err(|_| Error::IngressBind)?;
    let sent = client
        .send_to(&pkt[..pkt_len], ingress_addr)
        .map_err(|_| Error::IngressSend)?;
    let _ = writeln!(
        out,
        "[2] PETICIÓN UDP  {client_addr}  --{sent} B-->  {ingress_addr}   (std::net::UdpSocket::send_to)"
    );

    let mut arena = PacketArena::<4>::new();
    let recvd = ingress.recv_into(&mut arena).await?;
    let _ = writeln!(
        out,
        "[3] recv_into     slot index={} gen={}  len={}  src={}",
        recvd.slot.index(),
        recvd.slot.generation(),
        recvd.len,
        recvd.src
    );

    let result = pipe.ingest_slot(&arena, recvd.slot)?;
    let _ = writeln!(
        out,
        "[4] ingest_slot   reconstructed={}  destinos lógicos={:?}",
        result.reconstructed(),
        result.forward().dests()
    );

    let mut dests = [ingress_addr; MAX_FORWARD];
    let n = pipe.dest_addrs(&result.forward(), &mut dests)?;
    let _ = write!(out, "[5] dest_addrs    n={n}  [");
    for (i, addr) in dests.iter().take(n).enumerate() {
        if i > 0 {
            let _ = write!(out, ", ");
        }
        let _ = write!(out, "{addr}");
    }
    let _ = writeln!(out, "]");

    let fwd = ingress
        .forward_slot(&arena, recvd.slot, &dests[..n])
        .await?;
    let _ = writeln!(
        out,
        "[6] FANOUT UDP    forward_slot sent={fwd}  (IoBuf sobre el slot, sin Vec)"
    );

    let expected = arena.slot(recvd.slot)?;
    let na = recv_match(&child_a, expected)?;
    let nb = recv_match(&child_b, expected)?;
    let _ = writeln!(out, "[7] hijo A recibió {na} B  (coincide con arena.slot)");
    let _ = writeln!(out, "[7] hijo B recibió {nb} B  (coincide con arena.slot)");

    let m = pipe.metrics();
    let _ = writeln!(
        out,
        "[8] métricas      recv={} recon={} drop={}",
        m.received(),
        m.reconstructed(),
        m.dropped()
    );

    let _ = arena.release(recvd.slot);
    let _ = writeln!(out, "=== debug-udp ok ===");
    Ok(())
}
