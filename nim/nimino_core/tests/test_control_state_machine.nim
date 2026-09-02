import std/[options, unittest]

import nimino_core/domain/control_state_machine

proc elect(
    state: ControlState; term: uint64; candidate: string; supporters: seq[string]
): ControlState =
  let decision = settleControlPlan(
    planElection(
      state,
      ElectionRequest(term: term, candidateId: candidate, supporters: supporters),
    ),
    true,
  )
  doAssert decision.error == cseNone
  decision.state

proc append(
    state: ControlState; kind: ControlEntryKind; commandId: string;
    targetVoters: seq[string] = @[]
): ControlState =
  let decision = settleControlPlan(
    planAppend(
      state,
      AppendRequest(
        leaderId: state.leaderId,
        term: state.term,
        kind: kind,
        commandId: commandId,
        payload: commandId,
        targetVoters: targetVoters,
      ),
    ),
    true,
  )
  doAssert decision.error == cseNone
  decision.state

proc commit(
    state: ControlState; supporters: seq[string]
): ControlState =
  let decision = settleControlPlan(
    planCommit(
      state,
      CommitRequest(
        index: state.commitIndex + 1,
        leaderId: state.leaderId,
        term: state.term,
        supporters: supporters,
      ),
    ),
    true,
  )
  doAssert decision.error == cseNone
  decision.state

suite "Nimino replicated control state machine":
  test "votes reject stale logs and follower replication replaces only an uncommitted suffix":
    let initial = initControlState(@["node-a", "node-b", "node-c"])
    let staleVote = planVote(
      initial,
      VoteRequest(term: 1, candidateId: "node-a", lastIndex: 0, lastTerm: 0),
    )
    check staleVote.error == cseNone
    var leader = settleControlPlan(staleVote, true).state
    leader = elect(leader, 1, "node-a", @["node-a", "node-b"])
    leader = append(leader, cekCommand, "authoritative")
    check planVote(
      leader,
      VoteRequest(term: 2, candidateId: "node-b", lastIndex: 0, lastTerm: 0),
    ).error == cseCandidateLogStale

    var follower = elect(initial, 1, "node-a", @["node-a", "node-b"])
    follower = append(follower, cekCommand, "divergent")
    let replication = planReplication(
      follower,
      ReplicationRequest(
        leaderId: "node-a",
        term: 1,
        supporters: @["node-a", "node-b"],
        previousIndex: 0,
        entry: leader.log[0],
      ),
    )
    check replication.error == cseNone
    follower = settleControlPlan(replication, true).state
    check follower.log == leader.log
    check not checkControlQuorum(follower, @["node-a"]).granted
    check checkControlQuorum(follower, @["node-a", "node-c"]).granted

  test "minorities cannot elect or commit":
    let initial = initControlState(@["node-a", "node-b", "node-c"])
    let minorityElection = planElection(
      initial,
      ElectionRequest(term: 1, candidateId: "node-a", supporters: @["node-a"]),
    )
    check minorityElection.error == cseQuorumRequired
    check settleControlPlan(minorityElection, true).state == initial

    var state = elect(initial, 1, "node-a", @["node-a", "node-b"])
    state = append(state, cekCommand, "command-1")
    let minorityCommit = planCommit(
      state,
      CommitRequest(
        index: 1, leaderId: "node-a", term: 1, supporters: @["node-a"]
      ),
    )
    check minorityCommit.error == cseQuorumRequired
    check settleControlPlan(minorityCommit, true).state.commitIndex == 0

  test "voter authority changes only after two quorum commits":
    var state = initControlState(@["node-a", "node-b", "node-c"])
    state = elect(state, 1, "node-a", @["node-a", "node-b"])
    state = append(
      state,
      cekBeginJoint,
      "begin-joint",
      @["node-b", "node-c", "node-d"],
    )
    check state.phase == cvStableOld
    check state.voterEpoch == 1

    state = commit(state, @["node-a", "node-b"])
    check state.phase == cvJoint
    check state.voterEpoch == 2
    check state.leaderId.len == 0

    state = elect(state, 2, "node-b", @["node-a", "node-b", "node-c"])
    state = append(state, cekFinalize, "finalize")
    let oneSide = planCommit(
      state,
      CommitRequest(
        index: 2,
        leaderId: "node-b",
        term: 2,
        supporters: @["node-a", "node-b"],
      ),
    )
    check oneSide.error == cseQuorumRequired
    check settleControlPlan(oneSide, true).state.phase == cvJoint

    state = commit(state, @["node-a", "node-b", "node-c"])
    check state.phase == cvStableNew
    check state.voterEpoch == 3
    check state.commitIndex == 2

    state = settleControlPlan(planApply(state), true).state
    state = settleControlPlan(planApply(state), true).state
    state = settleControlPlan(planSnapshot(state, "stable-new-state"), true).state
    let recovered = recoverControlState(
      RecoveryInput(
        metadataRevision: state.metadataRevision,
        term: state.term,
        votedFor: state.votedFor,
        commitIndex: state.commitIndex,
        appliedIndex: state.appliedIndex,
        snapshot: state.snapshot,
      )
    )
    check recovered.error == cseNone
    check recovered.state.phase == cvStableNew
    check recovered.state.voterEpoch == 3
    check recovered.state.newVoters == @["node-b", "node-c", "node-d"]

  test "storage failure leaves the pre-transition state authoritative":
    var state = initControlState(@["node-a"])
    let electionPlan = planElection(
      state,
      ElectionRequest(term: 1, candidateId: "node-a", supporters: @["node-a"]),
    )
    let failedElection = settleControlPlan(electionPlan, false)
    check failedElection.error == cseStoreFailure
    check failedElection.state == state

    state = settleControlPlan(electionPlan, true).state
    let appendPlan = planAppend(
      state,
      AppendRequest(
        leaderId: "node-a",
        term: 1,
        kind: cekCommand,
        commandId: "command-1",
        payload: "payload",
      ),
    )
    let failedAppend = settleControlPlan(appendPlan, false)
    check failedAppend.error == cseStoreFailure
    check failedAppend.state == state

    state = settleControlPlan(appendPlan, true).state
    let commitPlan = planCommit(
      state,
      CommitRequest(
        index: 1, leaderId: "node-a", term: 1, supporters: @["node-a"]
      ),
    )
    let failedCommit = settleControlPlan(commitPlan, false)
    check failedCommit.error == cseStoreFailure
    check failedCommit.state.commitIndex == 0

  test "committed command ids replay exactly and reject conflicting content":
    var state = initControlState(@["node-a"])
    state = elect(state, 1, "node-a", @["node-a"])
    state = append(state, cekCommand, "command-1")
    state = commit(state, @["node-a"])

    let replay = planAppend(
      state,
      AppendRequest(
        leaderId: "node-a", term: 1, kind: cekCommand,
        commandId: "command-1", payload: "command-1",
      ),
    )
    check replay.effect == ceReplay
    check replay.error == cseNone
    check replay.actions.len == 0
    check replay.appliedEntry.get().index == 1

    var conflicting = replay.beforeState
    let conflict = planAppend(
      conflicting,
      AppendRequest(
        leaderId: "node-a", term: 1, kind: cekCommand,
        commandId: "command-1", payload: "different",
      ),
    )
    check conflict.error == cseCommandConflict

  test "snapshot recovery replays only the durable suffix":
    var state = initControlState(@["node-a"])
    state = elect(state, 1, "node-a", @["node-a"])
    state = append(state, cekCommand, "command-1")
    state = commit(state, @["node-a"])
    state = settleControlPlan(planApply(state), true).state
    state = settleControlPlan(planSnapshot(state, "applied-control-state"), true).state
    check state.snapshot.isSome
    check state.snapshot.get().lastIncludedIndex == 1
    check state.snapshot.get().statePayload == "applied-control-state"
    check state.log.len == 0

    state = append(state, cekCommand, "command-2")
    state = commit(state, @["node-a"])
    let recovery = recoverControlState(
      RecoveryInput(
        term: state.term,
        votedFor: state.votedFor,
        commitIndex: state.commitIndex,
        appliedIndex: state.appliedIndex,
        snapshot: state.snapshot,
        entries: state.log,
      )
    )
    check recovery.error == cseNone
    check recovery.state.commitIndex == 2
    check recovery.state.appliedIndex == 1
    check recovery.state.snapshot.get().lastIncludedIndex == 1
    check recovery.state.log.len == 1
    check recovery.state.log[0].commandId == "command-2"

  test "plans are deterministic and reject corrupt recovery prefixes":
    let state = initControlState(@["node-a", "node-b", "node-c"])
    let request = ElectionRequest(
      term: 1, candidateId: "node-b", supporters: @["node-a", "node-b"]
    )
    check planElection(state, request) == planElection(state, request)

    let corrupt = recoverControlState(
      RecoveryInput(
        term: 1,
        commitIndex: 2,
        appliedIndex: 0,
        entries: @[
          ControlEntry(
            index: 2,
            term: 1,
            voterEpoch: 1,
            kind: cekCommand,
            commandId: "gap",
          )
        ],
      )
    )
    check corrupt.error == cseCorruptRecovery
