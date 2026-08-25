pub(crate) fn canonical_node_id(bytes: [u8; 16]) -> [u8; 16] {
    *alopex_chirps::NodeId::from(bytes).as_bytes()
}
