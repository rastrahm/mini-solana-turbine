//! Arranque del validador: argumentos, cluster de ejemplo y flujo documentado.
//!
//! No abre el loop `io_uring` (eso bloquearía). Enlace mental:
//! `recv_into` → [`slot_queue`] → [`Pipeline::ingest_slot`] → [`Pipeline::dest_addrs`]
//! → send del slot (feature `uring`). Extra: métricas atómicas y firma opcional.

use mini_solana_turbine::pipeline::Pipeline;
use mini_solana_turbine::turbine::{self, Node, NodeId, Stake};
use mini_solana_turbine::{parse_addr, Error};
use std::env;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Purpose: `SocketAddr` de ejemplo `127.0.0.1:9000+id`.
/// Inputs: `id`.
/// Returns: dirección local.
fn peer_addr(id: u32) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000 + id as u16)
}

/// Purpose: `--bind=host:port` o `127.0.0.1:8001`.
/// Inputs: `std::env::args` (se consume en el bucle).
/// Returns: addr de bind, o `IngressBind`.
fn bind_from_args() -> Result<SocketAddr, Error> {
    for a in env::args().skip(1) {
        if let Some(rest) = a.strip_prefix("--bind=") {
            return parse_addr(rest);
        }
    }
    parse_addr("127.0.0.1:8001")
}

/// Purpose: Cluster de 4 nodos de demostración; id 1 es raíz.
/// Inputs: none.
/// Returns: árbol fanout 2.
fn demo_tree() -> Result<mini_solana_turbine::TurbineTree, Error> {
    turbine::tree::build(
        &[
            Node::new(NodeId::new(1), Stake::new(100), peer_addr(1)),
            Node::new(NodeId::new(2), Stake::new(50), peer_addr(2)),
            Node::new(NodeId::new(3), Stake::new(40), peer_addr(3)),
            Node::new(NodeId::new(4), Stake::new(10), peer_addr(4)),
        ],
        2,
    )
}

/// Purpose: Punto de entrada. Valida bind, construye pipeline, escribe el flujo.
/// Inputs: `--bind=ip:port` opcional.
/// Returns: `Ok(())` tras documentar; error de parseo/árbol/FEC.
fn main() -> Result<(), Error> {
    let bind = bind_from_args()?;
    let tree = demo_tree()?;
    let self_id = NodeId::new(1);
    let pipeline = Pipeline::with_defaults(tree, self_id)?;
    let m = pipeline.metrics();
    let _ = writeln!(
        io::stdout(),
        "mini-solana-turbine extra\n\
         bind (validado, no escuchando): {bind}\n\
         self: {:?}\n\
         métricas: recv={} recon={} drop={}\n\
         flujo: UDP recv_into(arena) -> slot_queue -> Pipeline::ingest_slot\n\
         ingest: [firma opcional] parse shred -> scratch FEC -> ForwardPlan\n\
         send: dest_addrs(plan) -> forward_slot (feature uring)\n\
         features: default=[uring,simd]",
        pipeline.self_id(),
        m.received(),
        m.reconstructed(),
        m.dropped()
    );
    Ok(())
}
