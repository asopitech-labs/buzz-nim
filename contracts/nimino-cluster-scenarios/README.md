# Nimino real-mesh lifecycle scenarios

Version 1 applies one deterministic contract to 1, 3, and 5 mTLS nodes. Every
case starts real Alopex Chirps UDP/QUIC runtimes, selects the failure node from
the fixed seed, exercises isolation, OS process kill, identity/incarnation
recovery, Nim-owned drain decisions, and confirms every socket can be rebound.

The isolated node has no discovery seed until rejoin. This is the portable CI
partition boundary: it exercises actual unreachable QUIC peers without
requiring host firewall or network-namespace privileges. Data convergence is
not inferred from transport reachability and remains owned by #59.

Run `just nimino-cluster-scenarios`. The successful run writes the uploadable
evidence artifact to `target/nim/nimino-cluster-scenarios.json`.
