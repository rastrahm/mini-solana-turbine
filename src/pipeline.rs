//! Pipeline lógico: parse → acumular set FEC → reconstruir → destinos Turbine.
//!
//! El envío UDP (feature `uring`) reutiliza los bytes del slot. Entre ingress y
//! este módulo: cola lock-free de [`SlotId`] ([`slot_queue`]).
//!
//! Requiere feature `simd`. Métricas en [`Metrics`]. Firma opcional del líder
//! vía [`Pipeline::require_leader`].

use crate::arena::{PacketArena, SlotId};
use crate::fec::{FecEngine, DEFAULT_SHARD_BYTES};
use crate::metrics::Metrics;
use crate::shred::{self, Shred, ShredPublicKey};
use crate::turbine::{NodeId, TurbineTree};
use crate::Error;
use crossbeam_channel::{Receiver, Sender};
use std::net::SocketAddr;

/// Máximo `k` / `n` de un [`Pipeline`] (arrays en stack al reconstruir).
pub const MAX_SHARDS: usize = 16;
/// Destinos máximos que caben en un [`ForwardPlan`] (fanout recortado).
pub const MAX_FORWARD: usize = 8;
/// Capacidad por defecto de [`slot_queue`].
pub const DEFAULT_SLOT_QUEUE: usize = 128;

/// Emisor lock-free de índices de slot (ingress → pipeline).
pub type SlotSender = Sender<SlotId>;
/// Receptor lock-free de índices de slot.
pub type SlotReceiver = Receiver<SlotId>;

/// Destinos Turbine para un shred ya parseado. No contiene payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardPlan {
    dests: [NodeId; MAX_FORWARD],
    count: u8,
}

/// Resultado de ingerir un shred: cuántos data shards se reconstruyeron + a quién reenviar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestResult {
    reconstructed: usize,
    forward: ForwardPlan,
}

/// Estado del validador local: engine FEC reutilizable, scratch del set, árbol, `self`.
pub struct Pipeline {
    engine: FecEngine,
    tree: TurbineTree,
    self_id: NodeId,
    original: Box<[u8]>,
    recovery: Box<[u8]>,
    orig_present: Box<[bool]>,
    rec_present: Box<[bool]>,
    restored: Box<[u8]>,
    fec_set_index: Option<u32>,
    metrics: Metrics,
    leader: Option<ShredPublicKey>,
}

impl ForwardPlan {
    /// Purpose: Plan vacío (hoja o aún no calculado).
    /// Inputs: none.
    /// Returns: `count == 0`.
    #[inline(always)]
    pub const fn empty() -> Self {
        Self {
            dests: [NodeId::new(0); MAX_FORWARD],
            count: 0,
        }
    }

    /// Purpose: Destinos a los que reenviar (fase 8).
    /// Inputs: none (`&self`).
    /// Returns: prefijo `0..count` de `dests`.
    #[inline(always)]
    pub fn dests(&self) -> &[NodeId] {
        &self.dests[..self.count as usize]
    }

    /// Purpose: Cuántos destinos hay.
    /// Inputs: none.
    /// Returns: `0..=MAX_FORWARD`.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.count as usize
    }

    /// Purpose: ¿Sin hijos en el árbol?
    /// Inputs: none.
    /// Returns: `count == 0`.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl IngestResult {
    /// Purpose: Data shards que el FEC acaba de llenar en el scratch.
    /// Inputs: none.
    /// Returns: 0 si aún no hay suficientes shards o no faltaba ninguno.
    #[inline(always)]
    pub const fn reconstructed(&self) -> usize {
        self.reconstructed
    }

    /// Purpose: Plan de fanout para el shred recién ingerido.
    /// Inputs: none.
    /// Returns: [`ForwardPlan`].
    #[inline(always)]
    pub const fn forward(&self) -> ForwardPlan {
        self.forward
    }
}

impl Pipeline {
    /// Purpose: Reserva scratch FEC y ata el árbol local.
    /// Inputs: `original_count` / `recovery_count` / `shard_bytes` — mismo contrato que
    ///   [`FecEngine`]; `tree` — cluster; `self_id` — este validador (debe existir).
    /// Returns: pipeline, o error de FEC / `TurbineUnknownNode`.
    pub fn new(
        original_count: usize,
        recovery_count: usize,
        shard_bytes: usize,
        tree: TurbineTree,
        self_id: NodeId,
    ) -> Result<Self, Error> {
        if original_count > MAX_SHARDS || recovery_count > MAX_SHARDS {
            return Err(Error::FecInconsistent);
        }
        let engine = FecEngine::new(original_count, recovery_count, shard_bytes)?;
        let _ = tree.node(self_id)?;
        let k = engine.original_count();
        let r = engine.recovery_count();
        let sb = engine.shard_bytes();
        Ok(Self {
            original: vec![0u8; k * sb].into_boxed_slice(),
            recovery: vec![0u8; r * sb].into_boxed_slice(),
            orig_present: vec![false; k].into_boxed_slice(),
            rec_present: vec![false; r].into_boxed_slice(),
            restored: vec![0u8; k * sb].into_boxed_slice(),
            engine,
            tree,
            self_id,
            fec_set_index: None,
            metrics: Metrics::new(),
            leader: None,
        })
    }

    /// Purpose: Pipeline de tests: `k=2`, `n=1`, shard 64.
    /// Inputs: `tree`, `self_id`.
    /// Returns: igual que [`new`](Self::new) con constantes por defecto.
    pub fn with_defaults(tree: TurbineTree, self_id: NodeId) -> Result<Self, Error> {
        Self::new(2, 1, DEFAULT_SHARD_BYTES, tree, self_id)
    }

    /// Purpose: Este validador.
    /// Inputs: none.
    /// Returns: [`NodeId`] pasado a `new`.
    #[inline(always)]
    pub fn self_id(&self) -> NodeId {
        self.self_id
    }

    /// Purpose: Contadores atómicos (`received` / `reconstructed` / `dropped`).
    /// Inputs: none.
    /// Returns: referencia a [`Metrics`] (lecturas `Relaxed`).
    #[inline(always)]
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Purpose: A partir de ahora `ingest` exige paquete `sig[64] || body`.
    /// Inputs: `pk` — pubkey educativa del líder.
    /// Returns: none.
    pub fn require_leader(&mut self, pk: ShredPublicKey) {
        self.leader = Some(pk);
    }

    /// Purpose: Vuelve al parseo sin firma (compatibilidad con fases 3–8).
    /// Inputs: none.
    /// Returns: none.
    pub fn clear_leader(&mut self) {
        self.leader = None;
    }

    /// Purpose: Traduce el [`ForwardPlan`] a `SocketAddr` de los hijos (sin heap).
    /// Inputs: `plan` — destinos lógicos; `out` — buffer del caller (`MAX_FORWARD` basta).
    /// Returns: cuántos addrs se escribieron (`min(plan.len(), out.len())`), o
    ///   `TurbineUnknownNode` si un id no está en el cluster.
    #[inline(always)]
    pub fn dest_addrs(&self, plan: &ForwardPlan, out: &mut [SocketAddr]) -> Result<usize, Error> {
        let n = plan.len().min(out.len());
        for (i, id) in plan.dests().iter().copied().enumerate().take(n) {
            out[i] = self.tree.node(id)?.addr();
        }
        Ok(n)
    }

    /// Purpose: Parsea `bytes`, guarda el payload en scratch y calcula destinos.
    /// Inputs: `bytes` — paquete completo (slot o buffer de test), no se clona a un `Vec`.
    ///   Si hay líder, debe ser `sig || body`.
    /// Returns: reconstrucciones + [`ForwardPlan`]; el payload se copia al scratch del set FEC.
    ///   Siempre incrementa `received`; en error también `dropped`.
    pub fn ingest_bytes(&mut self, bytes: &[u8]) -> Result<IngestResult, Error> {
        self.metrics.record_received();
        match self.ingest_bytes_inner(bytes) {
            Ok(result) => {
                self.metrics
                    .record_reconstructed(result.reconstructed as u64);
                Ok(result)
            }
            Err(err) => {
                self.metrics.record_dropped();
                Err(err)
            }
        }
    }

    /// Purpose: Parse (+ firma opcional) y FEC sin tocar contadores.
    /// Inputs: `bytes` — mismo contrato que [`ingest_bytes`].
    /// Returns: igual que `ingest_bytes` sin métricas.
    fn ingest_bytes_inner(&mut self, bytes: &[u8]) -> Result<IngestResult, Error> {
        let shred = match self.leader {
            Some(pk) => shred::parse_signed(bytes, &pk)?,
            None => shred::parse(bytes)?,
        };
        self.store_shred(shred)?;
        let reconstructed = self.try_reconstruct()?;
        let forward = self.plan_forward()?;
        Ok(IngestResult {
            reconstructed,
            forward,
        })
    }

    /// Purpose: Toma un slot de la arena y corre [`ingest_bytes`].
    /// Inputs: `arena` — pool; `slot` — handle vigente con `len` ya fijado.
    /// Returns: igual que `ingest_bytes`.
    pub fn ingest_slot<const SLOTS: usize>(
        &mut self,
        arena: &PacketArena<SLOTS>,
        slot: SlotId,
    ) -> Result<IngestResult, Error> {
        let bytes = arena.slot(slot)?;
        self.ingest_bytes(bytes)
    }

    /// Purpose: Copia el payload del shred al scratch del set.
    /// Inputs: `shred` — vista parseada (payload prestado).
    /// Returns: `Ok` si el set e índices cuadran.
    #[inline(always)]
    fn store_shred(&mut self, shred: Shred<'_>) -> Result<(), Error> {
        let header = shred.header();
        match self.fec_set_index {
            None => self.fec_set_index = Some(header.fec_set_index()),
            Some(id) if id == header.fec_set_index() => {}
            Some(_) => return Err(Error::FecInconsistent),
        }
        let payload = shred.payload();
        let sb = self.engine.shard_bytes();
        if payload.len() != sb {
            return Err(Error::FecInconsistent);
        }
        match shred {
            Shred::Data(_) => {
                let idx = header
                    .index()
                    .checked_sub(header.fec_set_index())
                    .ok_or(Error::ShredInvalidFec)? as usize;
                if idx >= self.engine.original_count() {
                    return Err(Error::ShredInvalidFec);
                }
                if self.orig_present[idx] {
                    return Err(Error::FecInconsistent);
                }
                let start = idx * sb;
                self.original[start..start + sb].copy_from_slice(payload);
                self.orig_present[idx] = true;
            }
            Shred::Code(cs) => {
                let idx = usize::from(cs.code_header().position());
                if idx >= self.engine.recovery_count() {
                    return Err(Error::ShredInvalidFec);
                }
                if self.rec_present[idx] {
                    return Err(Error::FecInconsistent);
                }
                let start = idx * sb;
                self.recovery[start..start + sb].copy_from_slice(payload);
                self.rec_present[idx] = true;
            }
        }
        Ok(())
    }

    /// Purpose: Si hay ≥ `k` shards, reconstruye los data faltantes en el scratch.
    /// Inputs: none (usa el set acumulado).
    /// Returns: cuántos originales se escribieron, o 0 si aún no alcanza.
    fn try_reconstruct(&mut self) -> Result<usize, Error> {
        let k = self.engine.original_count();
        let r = self.engine.recovery_count();
        let sb = self.engine.shard_bytes();
        let mut missing = 0usize;
        let mut present = 0usize;
        for &p in self.orig_present.iter() {
            if p {
                present += 1;
            } else {
                missing += 1;
            }
        }
        for &p in self.rec_present.iter() {
            if p {
                present += 1;
            }
        }
        if missing == 0 || present < k {
            return Ok(0);
        }

        let n = decode_scratch(
            &mut self.engine,
            &self.original,
            &self.recovery,
            &self.orig_present,
            &self.rec_present,
            &mut self.restored,
            k,
            r,
            sb,
        )?;
        for i in 0..k {
            if self.orig_present[i] {
                continue;
            }
            let start = i * sb;
            self.original[start..start + sb].copy_from_slice(&self.restored[start..start + sb]);
            self.orig_present[i] = true;
        }
        Ok(n)
    }

    /// Purpose: Hijos de `self` en el árbol (reenvío lógico).
    /// Inputs: none.
    /// Returns: hasta [`MAX_FORWARD`] destinos.
    #[inline(always)]
    fn plan_forward(&self) -> Result<ForwardPlan, Error> {
        let mut dests = [NodeId::new(0); MAX_FORWARD];
        let n = self.tree.children_of(self.self_id, &mut dests)?;
        Ok(ForwardPlan {
            dests,
            count: n as u8,
        })
    }

    /// Purpose: Payload data del scratch si ya está presente (recibido o reconstruido).
    /// Inputs: `index` — `0..k` dentro del set FEC.
    /// Returns: slice de `shard_bytes`, o `FecInconsistent` si aún no está.
    pub fn original_shard(&self, index: usize) -> Result<&[u8], Error> {
        if index >= self.engine.original_count() || !self.orig_present[index] {
            return Err(Error::FecInconsistent);
        }
        let sb = self.engine.shard_bytes();
        let start = index * sb;
        Ok(&self.original[start..start + sb])
    }
}

/// Purpose: Llama a [`FecEngine::decode`] con slices sobre scratch prealocado.
/// Inputs: buffers del pipeline y conteos `k`/`r`/`sb`.
/// Returns: originales restaurados.
#[allow(clippy::too_many_arguments)]
fn decode_scratch(
    engine: &mut FecEngine,
    original: &[u8],
    recovery: &[u8],
    orig_present: &[bool],
    rec_present: &[bool],
    restored: &mut [u8],
    k: usize,
    r: usize,
    sb: usize,
) -> Result<usize, Error> {
    let mut original_opt: [Option<&[u8]>; MAX_SHARDS] = [None; MAX_SHARDS];
    let mut recovery_opt: [Option<&[u8]>; MAX_SHARDS] = [None; MAX_SHARDS];
    for i in 0..k {
        if orig_present[i] {
            let start = i * sb;
            original_opt[i] = Some(&original[start..start + sb]);
        }
    }
    for i in 0..r {
        if rec_present[i] {
            let start = i * sb;
            recovery_opt[i] = Some(&recovery[start..start + sb]);
        }
    }

    let mut parts: [&mut [u8]; MAX_SHARDS] = empty_mut_shards();
    let base = restored.as_mut_ptr();
    for (i, part) in parts.iter_mut().enumerate().take(k) {
        // SAFETY: `restored` es `k * sb` bytes; cada rango `i*sb..(i+1)*sb` es
        // disjunto, y no hay otro `&mut` a ese tramo en este stack frame.
        *part = unsafe { core::slice::from_raw_parts_mut(base.add(i * sb), sb) };
    }
    engine.decode(&original_opt[..k], &recovery_opt[..r], &mut parts[..k])
}

/// Purpose: Array de slices vacíos para inicializar `parts` antes de rellenar.
/// Inputs: none.
/// Returns: `[ &mut []; MAX_SHARDS ]`.
fn empty_mut_shards<'a>() -> [&'a mut [u8]; MAX_SHARDS] {
    fn one<'a>() -> &'a mut [u8] {
        &mut []
    }
    [(); MAX_SHARDS].map(|_| one())
}

/// Purpose: Cola lock-free `SlotId` (capacidad acotada).
/// Inputs: `capacity` — slots en vuelo; si es 0 se usa [`DEFAULT_SLOT_QUEUE`].
/// Returns: par sender/receiver `crossbeam-channel` bounded.
pub fn slot_queue(capacity: usize) -> (SlotSender, SlotReceiver) {
    let cap = if capacity == 0 {
        DEFAULT_SLOT_QUEUE
    } else {
        capacity
    };
    crossbeam_channel::bounded(cap)
}
