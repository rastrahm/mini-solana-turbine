# Diagrama de clases — tipos del crate

Relaciones entre structs/enums públicos (y los internos de ingress relevantes). Rust no es OOP: “clase” aquí = tipo con API y composición.

```mermaid
classDiagram
    direction TB

    class Error {
        <<enumeration>>
        ShredTruncated
        ShredInvalidType
        ShredBadSignature
        ArenaExhausted
        FecTooManyErasures
        IngressBind
        IngressRecv
        IngressSend
        TurbineUnknownNode
        TurbineEmptyCluster
    }

    class PacketArena {
        <<const SLOTS>>
        -storage: Box~u8~
        -lengths: u16[]
        -occupied: bool[]
        -generations: u16[]
        -free: u16[]
        -free_top: u16
        +new() PacketArena
        +acquire() SlotId
        +release(SlotId)
        +slot(SlotId) ~u8~
        +slot_mut(SlotId) ~u8~
        +set_len(SlotId, usize)
        +slot_mut_ptr(SlotId) *mut u8
    }

    class SlotId {
        +index: u16
        +generation: u16
    }

    class RecvDatagram {
        +slot: SlotId
        +src: SocketAddr
        +len: usize
    }

    class UdpIngress {
        -socket: UdpSocket
        +bind(addr) UdpIngress
        +recv_into(arena) RecvDatagram
        +send_slot(arena, slot, dest) usize
        +forward_slot(arena, slot, dests) usize
    }

    class ShredHeader {
        <<repr C packed>>
        slot: u64
        fec_set_index: u32
        index: u32
        version: u16
        shred_type: u8
    }

    class DataShred {
        header: ShredHeader
        data: DataShredHeader
        payload: ~u8~
    }

    class CodeShred {
        header: ShredHeader
        code: CodeShredHeader
        payload: ~u8~
    }

    class Shred {
        <<enumeration>>
        Data(DataShred)
        Code(CodeShred)
    }

    class ShredPublicKey {
        bytes: u8[32]
    }

    class ShredSecretKey {
        bytes: u8[32]
        +public() ShredPublicKey
    }

    class FecEngine {
        -encoder
        -decoder
        -original_count: usize
        -recovery_count: usize
        -shard_bytes: usize
        +new(k, n, sb) FecEngine
        +encode(original, recovery_out)
        +decode(original, recovery, restored) usize
    }

    class NodeId {
        raw: u32
    }

    class Stake {
        amount: u64
    }

    class Node {
        id: NodeId
        stake: Stake
        addr: SocketAddr
    }

    class TurbineTree {
        -nodes: Box~Node~
        -fanout: u8
        +build(nodes, fanout) TurbineTree
        +root() NodeId
        +children_of(id, out) usize
        +node(id) Node
    }

    class ForwardPlan {
        -dests: NodeId[8]
        -count: u8
        +dests() ~NodeId~
        +is_empty() bool
    }

    class IngestResult {
        -reconstructed: usize
        -forward: ForwardPlan
    }

    class Metrics {
        -received: AtomicU64
        -reconstructed: AtomicU64
        -dropped: AtomicU64
        +record_received()
        +record_reconstructed(n)
        +record_dropped()
        +snapshot() MetricsSnapshot
    }

    class Pipeline {
        -engine: FecEngine
        -tree: TurbineTree
        -self_id: NodeId
        -original: Box~u8~
        -recovery: Box~u8~
        -metrics: Metrics
        -leader: Option~ShredPublicKey~
        +with_defaults(tree, self_id) Pipeline
        +ingest_bytes(bytes) IngestResult
        +ingest_slot(arena, slot) IngestResult
        +dest_addrs(plan, out) usize
        +require_leader(pk)
        +metrics() Metrics
    }

    PacketArena ..> SlotId : produce
    UdpIngress ..> PacketArena : recv/send sobre slots
    UdpIngress ..> RecvDatagram : produce
    RecvDatagram --> SlotId

    Shred --> DataShred
    Shred --> CodeShred
    DataShred --> ShredHeader
    CodeShred --> ShredHeader
    ShredSecretKey ..> ShredPublicKey : public()

    TurbineTree *-- Node
    Node --> NodeId
    Node --> Stake

    Pipeline *-- FecEngine
    Pipeline *-- TurbineTree
    Pipeline *-- Metrics
    Pipeline --> ShredPublicKey : leader opcional
    Pipeline ..> Shred : parse
    Pipeline ..> ForwardPlan : plan_forward
    Pipeline ..> IngestResult : produce
    IngestResult --> ForwardPlan
    ForwardPlan --> NodeId

    PacketArena ..> Error : errores
    Pipeline ..> Error : errores
    UdpIngress ..> Error : errores
    FecEngine ..> Error : errores
    TurbineTree ..> Error : errores
```

## Dependencias por feature

```mermaid
flowchart TB
    subgraph Siempre
        arena
        shred
        error
        metrics
        turbine
        ingress_parse["ingress::parse_addr"]
    end

    subgraph uring["feature uring"]
        UdpIngress
    end

    subgraph simd["feature simd"]
        fec
        pipeline
    end

    pipeline --> fec
    pipeline --> turbine
    pipeline --> shred
    pipeline --> arena
    pipeline --> metrics
    UdpIngress --> arena
```

## Notas

- `Pipeline` **posee** el `TurbineTree` (move en `with_defaults`).
- `FecEngine` del pipeline **no** es el temporal de `debug_udp` (ese se crea solo para rellenar `r0` de demo).
- Payload de `Shred` es un **préstamo** (`&[u8]`) sobre el slot, no un dueño.
