# Flujograma — arquitectura del pipeline

Vista de extremo a extremo: un shred entra por UDP, vive en un slot de arena y sale hacia los hijos del árbol Turbine **sin clonar el payload**.

```mermaid
flowchart LR
    subgraph Red["Red UDP"]
        Peer["Peer / líder<br/>(std send_to)"]
        Hijos["Hijos Turbine<br/>(nodos 2..n)"]
    end

    subgraph Ingress["ingress (feature uring)"]
        U["UdpIngress"]
        Recv["recv_into<br/>SQE → CQE"]
        Send["forward_slot<br/>send_slot"]
    end

    subgraph Memoria["Memoria fija"]
        Arena["PacketArena<br/>1× Box, N slots × 1228 B"]
        Slot["SlotId<br/>index + generation"]
    end

    subgraph Cola["Lock-free"]
        Q["slot_queue<br/>crossbeam-channel"]
    end

    subgraph Pipe["pipeline (feature simd)"]
        P["Pipeline"]
        Parse["parse / parse_signed"]
        FEC["FecEngine<br/>scratch + decode"]
        Plan["ForwardPlan"]
        M["Metrics<br/>AtomicU64"]
    end

    subgraph Turbine["turbine"]
        Tree["TurbineTree<br/>stake → heap k-ario"]
    end

    Peer -->|datagrama| U
    U --> Recv
    Recv -->|escribe bytes| Arena
    Arena --> Slot
    Slot --> Q
    Q --> P
    P --> Parse
    Parse --> FEC
    P --> Tree
    Tree --> Plan
    P --> M
    Plan -->|dest_addrs| Send
    Send -->|mismos bytes del slot| Hijos
    Arena -.->|IoBuf ptr| Recv
    Arena -.->|IoBuf ptr| Send
```

## Lectura rápida

1. **Peer** manda un datagrama al socket uring.
2. **`recv_into`** hace `acquire` + SQE; el kernel escribe en el slot.
3. Se pasa un **`SlotId`** (no el `Vec` del paquete) por la cola.
4. **`Pipeline`** parsea (firma opcional), acumula FEC y arma el plan de hijos.
5. **`forward_slot`** reenvía el mismo tramo de `storage` a cada `SocketAddr`.
6. **`release`** devuelve el slot a la free-list.

## Qué no clona

```text
storage[i * PACKET_SIZE ..]
        ▲
        │  RecvSlot / SendSlot (puntero)
        │
   kernel io_uring
```

Un solo backing en `PacketArena::new`. Hot path = índices y slices.
