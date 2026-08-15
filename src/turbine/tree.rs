//! Cálculo del árbol Turbine (stake-weighted fanout).
//!
//! Layout: heap k-ario sobre el cluster **ordenado**.
//!
//! 1. Orden: stake descendente; empate → [`NodeId`] ascendente (como un pubkey).
//! 2. El índice 0 es la raíz (más stake; o el `NodeId` menor si empatan).
//! 3. Hijos del índice `i` con fanout `f`: `f*i+1 .. f*i+f` (si existen).
//!
//! `build` puede asignar (una `Box` del cluster). [`TurbineTree::children_of`]
//! no asigna: solo aritmética sobre esos índices.

use crate::Error;
use core::fmt;
use std::net::SocketAddr;

/// Fanout por defecto (binario). Solana usa valores mayores en TVU; aquí 2 basta para tests.
pub const DEFAULT_FANOUT: u8 = 2;

/// Identidad de un validador (sustituto de pubkey de 32 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

/// Stake en unidades abstractas (`u64`, analogía a lamports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stake(u64);

/// Miembro del cluster: id, stake y dirección de reenvío (aún no se envía en esta fase).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    id: NodeId,
    stake: Stake,
    addr: SocketAddr,
}

/// Árbol de reenvío: nodos ya ordenados + fanout. Sin locks.
pub struct TurbineTree {
    nodes: Box<[Node]>,
    fanout: u8,
}

impl NodeId {
    /// Purpose: Newtype de identidad.
    /// Inputs: `raw` — entero estable en tests (p. ej. 1, 2, 3).
    /// Returns: [`NodeId`].
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Purpose: Valor interno.
    /// Inputs: `self`.
    /// Returns: `u32`.
    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl Stake {
    /// Purpose: Newtype de stake.
    /// Inputs: `amount` — mayor = más cerca de la raíz (salvo empate).
    /// Returns: [`Stake`].
    pub const fn new(amount: u64) -> Self {
        Self(amount)
    }

    /// Purpose: Magnitud del stake.
    /// Inputs: `self`.
    /// Returns: `u64`.
    #[inline(always)]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Node {
    /// Purpose: Construye un miembro del cluster.
    /// Inputs: `id` — identidad; `stake` — peso; `addr` — destino UDP futuro.
    /// Returns: [`Node`].
    pub fn new(id: NodeId, stake: Stake, addr: SocketAddr) -> Self {
        Self { id, stake, addr }
    }

    /// Purpose: Identidad del nodo.
    /// Inputs: `self`.
    /// Returns: [`NodeId`].
    #[inline(always)]
    pub const fn id(self) -> NodeId {
        self.id
    }

    /// Purpose: Stake del nodo.
    /// Inputs: `self`.
    /// Returns: [`Stake`].
    #[inline(always)]
    pub const fn stake(self) -> Stake {
        self.stake
    }

    /// Purpose: Dirección de reenvío.
    /// Inputs: `self`.
    /// Returns: [`SocketAddr`].
    #[inline(always)]
    pub const fn addr(self) -> SocketAddr {
        self.addr
    }
}

impl TurbineTree {
    /// Purpose: Fanout compilado en este árbol.
    /// Inputs: none (`&self`).
    /// Returns: `f` de `build`. `0` implica que nadie tiene hijos.
    #[inline(always)]
    pub fn fanout(&self) -> u8 {
        self.fanout
    }

    /// Purpose: Número de nodos.
    /// Inputs: none.
    /// Returns: `>= 1` si `build` tuvo éxito.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Purpose: ¿Cluster vacío? Tras `build` Ok es siempre `false`.
    /// Inputs: none.
    /// Returns: `nodes.is_empty()`.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Purpose: Raíz = primer nodo del orden (máximo stake).
    /// Inputs: none.
    /// Returns: [`NodeId`] de `nodes[0]`.
    #[inline(always)]
    pub fn root(&self) -> NodeId {
        self.nodes[0].id
    }

    /// Purpose: Datos del nodo si pertenece al árbol.
    /// Inputs: `id` — identidad buscada.
    /// Returns: [`Node`] copiado, o `TurbineUnknownNode`.
    #[inline(always)]
    pub fn node(&self, id: NodeId) -> Result<Node, Error> {
        Ok(self.nodes[self.index_of(id)?])
    }

    /// Purpose: Padre en el heap k-ario.
    /// Inputs: `id` — nodo existente.
    /// Returns: `None` si es raíz; `Some(parent)` si no; `TurbineUnknownNode` si no está.
    #[inline(always)]
    pub fn parent_of(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        let i = self.index_of(id)?;
        if i == 0 || self.fanout == 0 {
            return Ok(None);
        }
        let parent = (i - 1) / usize::from(self.fanout);
        Ok(Some(self.nodes[parent].id))
    }

    /// Purpose: Escribe los hijos de `id` en `out` (sin heap).
    /// Inputs: `id` — padre; `out` — buffer (típicamente tamaño `fanout`).
    /// Returns: cuántos hijos caben en `out` (puede ser 0: hoja).
    ///   Si `out` es más corto que el fanout real, se trunca.
    #[inline(always)]
    pub fn children_of(&self, id: NodeId, out: &mut [NodeId]) -> Result<usize, Error> {
        let i = self.index_of(id)?;
        if self.fanout == 0 {
            return Ok(0);
        }
        let f = usize::from(self.fanout);
        let start = i
            .checked_mul(f)
            .and_then(|v| v.checked_add(1))
            .ok_or(Error::TurbineUnknownNode)?;
        let mut n = 0usize;
        while n < f && n < out.len() {
            let child = start + n;
            if child >= self.nodes.len() {
                break;
            }
            out[n] = self.nodes[child].id;
            n += 1;
        }
        Ok(n)
    }

    /// Purpose: Un nodo es hoja si el primer hijo teórico cae fuera del array.
    /// Inputs: `id`.
    /// Returns: `true` si no hay índices `f*i+1`.
    #[inline(always)]
    pub fn is_leaf(&self, id: NodeId) -> Result<bool, Error> {
        let i = self.index_of(id)?;
        if self.fanout == 0 {
            return Ok(true);
        }
        let start = i
            .checked_mul(usize::from(self.fanout))
            .and_then(|v| v.checked_add(1))
            .ok_or(Error::TurbineUnknownNode)?;
        Ok(start >= self.nodes.len())
    }

    /// Purpose: Índice del nodo en el array ordenado.
    /// Inputs: `id`.
    /// Returns: `0..len` o `TurbineUnknownNode`.
    #[inline(always)]
    fn index_of(&self, id: NodeId) -> Result<usize, Error> {
        self.nodes
            .iter()
            .position(|n| n.id == id)
            .ok_or(Error::TurbineUnknownNode)
    }
}

/// Purpose: Ordena el cluster y materializa el heap k-ario.
/// Inputs: `nodes` — miembros (se copian); `fanout` — hijos máximos por nodo.
/// Returns: árbol con raíz = mayor stake; `TurbineEmptyCluster` si `nodes` está vacío.
pub fn build(nodes: &[Node], fanout: u8) -> Result<TurbineTree, Error> {
    if nodes.is_empty() {
        return Err(Error::TurbineEmptyCluster);
    }
    let mut ordered = nodes.to_vec();
    ordered.sort_by(|a, b| b.stake.cmp(&a.stake).then(a.id.cmp(&b.id)));
    Ok(TurbineTree {
        nodes: ordered.into_boxed_slice(),
        fanout,
    })
}

impl fmt::Debug for TurbineTree {
    /// Purpose: Debug sin volcar addrs de más: ids en orden y fanout.
    /// Inputs: `f`.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurbineTree")
            .field("fanout", &self.fanout)
            .field("len", &self.nodes.len())
            .field("root", &self.root())
            .finish()
    }
}

impl PartialEq for TurbineTree {
    /// Purpose: Igualdad por fanout y secuencia de nodos ordenados.
    /// Inputs: `other`.
    /// Returns: `bool`.
    fn eq(&self, other: &Self) -> bool {
        self.fanout == other.fanout && self.nodes.as_ref() == other.nodes.as_ref()
    }
}

impl Eq for TurbineTree {}

#[cfg(test)]
mod tests {
    use super::{build, Node, NodeId, Stake, DEFAULT_FANOUT};
    use crate::Error;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    /// Purpose: Addr local determinista por id.
    /// Inputs: `id` — usado como puerto `8000 + id`.
    /// Returns: `127.0.0.1:…`.
    fn addr(id: u32) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8000 + id as u16)
    }

    /// Purpose: Atajo de nodo.
    /// Inputs: `id`, `stake`.
    /// Returns: [`Node`].
    fn node(id: u32, stake: u64) -> Node {
        Node::new(NodeId::new(id), Stake::new(stake), addr(id))
    }

    /// Purpose: Cluster vacío no forma árbol.
    /// Inputs: none.
    /// Returns: panics si no es `TurbineEmptyCluster`.
    #[test]
    fn empty_cluster_is_error() {
        assert_eq!(build(&[], DEFAULT_FANOUT), Err(Error::TurbineEmptyCluster));
    }

    /// Purpose: Un solo nodo es raíz y hoja.
    /// Inputs: none.
    /// Returns: panics si tiene padre o hijos.
    #[test]
    fn single_node_is_root_and_leaf() {
        let tree = build(&[node(7, 100)], 2).expect("tree");
        assert_eq!(tree.root(), NodeId::new(7));
        assert_eq!(tree.parent_of(NodeId::new(7)), Ok(None));
        let mut kids = [NodeId::new(0); 2];
        assert_eq!(tree.children_of(NodeId::new(7), &mut kids), Ok(0));
        assert_eq!(tree.is_leaf(NodeId::new(7)), Ok(true));
    }

    /// Purpose: Fanout 2, heap binario por stake (A>B>C>D>E>F>G).
    /// Inputs: none.
    /// Returns: panics si los hijos no coinciden con el layout.
    #[test]
    fn fanout_2_children() {
        let nodes = [
            node(1, 70),
            node(2, 60),
            node(3, 50),
            node(4, 40),
            node(5, 30),
            node(6, 20),
            node(7, 10),
        ];
        let tree = build(&nodes, 2).expect("tree");
        assert_eq!(tree.root(), NodeId::new(1));
        let mut kids = [NodeId::new(0); 2];
        assert_eq!(tree.children_of(NodeId::new(1), &mut kids), Ok(2));
        assert_eq!(&kids, &[NodeId::new(2), NodeId::new(3)]);
        assert_eq!(tree.children_of(NodeId::new(2), &mut kids), Ok(2));
        assert_eq!(&kids, &[NodeId::new(4), NodeId::new(5)]);
        assert_eq!(tree.children_of(NodeId::new(3), &mut kids), Ok(2));
        assert_eq!(&kids, &[NodeId::new(6), NodeId::new(7)]);
        assert_eq!(tree.children_of(NodeId::new(4), &mut kids), Ok(0));
        assert_eq!(tree.is_leaf(NodeId::new(7)), Ok(true));
        assert_eq!(tree.is_leaf(NodeId::new(1)), Ok(false));
        assert_eq!(tree.parent_of(NodeId::new(5)), Ok(Some(NodeId::new(2))));
    }

    /// Purpose: Fanout 3: la raíz tiene 3 hijos; el siguiente cubre el resto.
    /// Inputs: none.
    /// Returns: panics si el ternario no cuadra.
    #[test]
    fn fanout_3_children() {
        let nodes = [
            node(1, 70),
            node(2, 60),
            node(3, 50),
            node(4, 40),
            node(5, 30),
            node(6, 20),
            node(7, 10),
        ];
        let tree = build(&nodes, 3).expect("tree");
        let mut kids = [NodeId::new(0); 3];
        assert_eq!(tree.children_of(tree.root(), &mut kids), Ok(3));
        assert_eq!(&kids, &[NodeId::new(2), NodeId::new(3), NodeId::new(4)]);
        assert_eq!(tree.children_of(NodeId::new(2), &mut kids), Ok(3));
        assert_eq!(&kids, &[NodeId::new(5), NodeId::new(6), NodeId::new(7)]);
        assert_eq!(tree.children_of(NodeId::new(3), &mut kids), Ok(0));
    }

    /// Purpose: Mismo stake → gana el `NodeId` menor (orden lexicográfico).
    /// Inputs: none.
    /// Returns: panics si la raíz no es el id más chico.
    #[test]
    fn equal_stake_breaks_ties_by_node_id() {
        let tree = build(&[node(9, 50), node(3, 50), node(6, 50)], 2).expect("tree");
        assert_eq!(tree.root(), NodeId::new(3));
        let mut kids = [NodeId::new(0); 2];
        assert_eq!(tree.children_of(NodeId::new(3), &mut kids), Ok(2));
        assert_eq!(&kids, &[NodeId::new(6), NodeId::new(9)]);
    }

    /// Purpose: Ordenar no muta la entrada; el stake alto queda raíz aunque se pase al final.
    /// Inputs: none.
    /// Returns: panics si la raíz no es el de 100.
    #[test]
    fn sort_is_deterministic_regardless_of_input_order() {
        let a = [node(1, 1), node(2, 100), node(3, 50)];
        let b = [node(3, 50), node(1, 1), node(2, 100)];
        let ta = build(&a, 2).expect("a");
        let tb = build(&b, 2).expect("b");
        assert_eq!(ta.root(), NodeId::new(2));
        assert_eq!(tb.root(), ta.root());
    }

    /// Purpose: Id ausente.
    /// Inputs: none.
    /// Returns: panics si no es `TurbineUnknownNode`.
    #[test]
    fn unknown_node() {
        let tree = build(&[node(1, 1)], 2).expect("tree");
        assert_eq!(
            tree.children_of(NodeId::new(99), &mut []),
            Err(Error::TurbineUnknownNode)
        );
        assert_eq!(
            tree.parent_of(NodeId::new(99)),
            Err(Error::TurbineUnknownNode)
        );
    }

    /// Purpose: `node()` expone addr y stake copiados.
    /// Inputs: none.
    /// Returns: panics si el addr no es el de `id`.
    #[test]
    fn node_exposes_addr() {
        let tree = build(&[node(4, 8)], 2).expect("tree");
        let n = tree.node(NodeId::new(4)).expect("node");
        assert_eq!(n.stake(), Stake::new(8));
        assert_eq!(n.addr(), addr(4));
    }
}
