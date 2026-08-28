# Nimino WSL Chirps certification v1

This contract certifies the existing `nimino-chirps` adapter on a real WSL2
filesystem under `/home`. It does not add another cluster layer. Chirps remains
limited to authenticated negotiation, membership facts, and opaque transport;
database, replication, sync, quorum, and product policy remain Nimino-owned.

`just wsl-chirps-contract` is the platform-independent source gate. On WSL,
`just wsl-chirps-certify` runs the real UDP/QUIC tests with all temporary
identity and certificate material on the distribution filesystem, then writes
`target/nim/nimino-wsl-chirps.json`.

The runtime proof covers stable node identity, private-key permissions, mutual
trust rejection, certificate reload, bind-address change and rejoin, graceful
shutdown, and socket release. It creates no Buzz compatibility mode.
