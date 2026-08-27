# Nimino membership policy v1

`domain.membership.policy` owns four pure decisions:

- channel join/add/role-change/remove/leave, including agent-owner authority;
- relay-roster add/role-change/remove;
- durable invite mint/claim;
- atomic community ownership transfer.

Rust supplies verified identity, role, clock, crypto, quota, and locked database
facts, then executes the selected effect. Issue #12 removes the listed duplicate
Rust branches and the v1 stateless invite drain path. No compatibility runtime
or fallback is permitted.
