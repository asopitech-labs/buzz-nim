//! Narrow dependency boundary around Alopex Chirps.
//!
//! Chirps supplies node negotiation, membership facts, and secure messaging.
//! Nimino owns quorum, replication, conflict, storage, and product policy.

#![deny(missing_docs)]

mod upstream;

/// Opaque cluster-node identity without exposing the upstream crate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId([u8; 16]);

impl NodeId {
    /// Constructs a node identity from its stable 16-byte representation.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(upstream::canonical_node_id(bytes))
    }

    /// Returns the stable 16-byte representation.
    pub fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_round_trips_without_exposing_chirps() {
        let bytes = [0x2a; 16];
        assert_eq!(NodeId::from_bytes(bytes).as_bytes(), bytes);
    }
}
