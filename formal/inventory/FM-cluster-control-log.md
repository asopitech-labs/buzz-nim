# FM-cluster-control-log

- Purpose: fix the Nimino v1 quorum, election epoch, joint voter transition,
  commit, snapshot, and recovery safety contract for Issue #46.
- Target paths: future runtime owners #48, #49, #51, and #52; transport adapter
  `crates/nimino-chirps/` is observational input only.
- Invariants: authority and commit require active-phase quorum; quorum
  certificates intersect; voter transitions are committed and sequential;
  terms/epochs do not regress; snapshot/recovery never exceed commit.
- Tool: TLA+/TLC, because ordering, timeout, voter transition, commit, crash,
  and replay are temporal state transitions.
- Model: `formal/tla/cluster/NiminoControlLog.tla`
- Scenario: `formal/scenarios/NiminoControlLog_3Node.cfg`
- Verification: `just control-model-check`
- Evidence: `formal/evidence/FM-cluster-control-log_20260901_summary.md`
- Status: active
- Change policy: update the versioned JSON contract, rerun TLC, update hashes
  and evidence, then run `just control-model-contract`.
- Retirement: only an explicit, model-checked successor control protocol may
  retire v1; the incompatible publication remains owned by Issue #12.
