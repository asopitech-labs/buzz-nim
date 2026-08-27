---- MODULE NiminoControlLog ----
EXTENDS Naturals, FiniteSets

CONSTANTS Nodes, OldVoters, NewVoters,
          MaxLogIndex, MaxTerm, MaxClock, ElectionTimeout

ASSUME /\ IsFiniteSet(Nodes)
       /\ OldVoters \subseteq Nodes
       /\ NewVoters \subseteq Nodes
       /\ Cardinality(OldVoters) > 0
       /\ Cardinality(NewVoters) > 0
       /\ MaxLogIndex \in Nat \ {0}
       /\ MaxTerm \in Nat \ {0}
       /\ MaxClock \in Nat
       /\ ElectionTimeout \in Nat \ {0}

StableOld == "stable-old"
Joint == "joint"
StableNew == "stable-new"
NoPhase == "none"
Phases == {StableOld, Joint, StableNew}

Command == "command"
BeginJoint == "begin-joint"
Finalize == "finalize"
Empty == "empty"
EntryKinds == {Command, BeginJoint, Finalize, Empty}
LogSlots == 1..MaxLogIndex

Majority(voters) == (Cardinality(voters) \div 2) + 1

HasQuorum(currentPhase, supporters) ==
    CASE currentPhase = StableOld ->
             Cardinality(supporters \cap OldVoters) >= Majority(OldVoters)
      [] currentPhase = Joint ->
             /\ Cardinality(supporters \cap OldVoters) >= Majority(OldVoters)
             /\ Cardinality(supporters \cap NewVoters) >= Majority(NewVoters)
      [] currentPhase = StableNew ->
             Cardinality(supporters \cap NewVoters) >= Majority(NewVoters)

AllowedKinds(currentPhase) ==
    CASE currentPhase = StableOld -> {Command, BeginJoint}
      [] currentPhase = Joint -> {Command, Finalize}
      [] currentPhase = StableNew -> {Command}

IsConfigurationChange(kind) == kind \in {BeginJoint, Finalize}

PhaseAfter(currentPhase, kind) ==
    CASE kind = BeginJoint -> Joint
      [] kind = Finalize -> StableNew
      [] OTHER -> currentPhase

VoterEpochAfter(currentEpoch, kind) ==
    IF IsConfigurationChange(kind) THEN currentEpoch + 1 ELSE currentEpoch

VARIABLES phase, term, voterEpoch,
          logicalClock, electionDeadline,
          votes, leaderElected, electionProofPhase, electionProofVotes,
          lastIndex, commitIndex, appliedIndex,
          entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
          proofPhases, proofAcks, phaseAt, voterEpochAt,
          snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase

vars == <<phase, term, voterEpoch,
          logicalClock, electionDeadline,
          votes, leaderElected, electionProofPhase, electionProofVotes,
          lastIndex, commitIndex, appliedIndex,
          entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
          proofPhases, proofAcks, phaseAt, voterEpochAt,
          snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

Init ==
    /\ phase = StableOld
    /\ term = 1
    /\ voterEpoch = 1
    /\ logicalClock = 0
    /\ electionDeadline = ElectionTimeout
    /\ votes = {}
    /\ leaderElected = FALSE
    /\ electionProofPhase = NoPhase
    /\ electionProofVotes = {}
    /\ lastIndex = 0
    /\ commitIndex = 0
    /\ appliedIndex = 0
    /\ entryKinds = [i \in LogSlots |-> Empty]
    /\ entryTerms = [i \in LogSlots |-> 0]
    /\ entryVoterEpochs = [i \in LogSlots |-> 0]
    /\ acknowledgements = [i \in LogSlots |-> {}]
    /\ proofPhases = [i \in LogSlots |-> NoPhase]
    /\ proofAcks = [i \in LogSlots |-> {}]
    /\ phaseAt = [i \in LogSlots |-> NoPhase]
    /\ voterEpochAt = [i \in LogSlots |-> 0]
    /\ snapshotIndex = 0
    /\ snapshotTerm = 0
    /\ snapshotVoterEpoch = 1
    /\ snapshotPhase = StableOld

Tick ==
    /\ logicalClock < MaxClock
    /\ logicalClock' = logicalClock + 1
    /\ UNCHANGED <<phase, term, voterEpoch, electionDeadline,
                   votes, leaderElected, electionProofPhase, electionProofVotes,
                   lastIndex, commitIndex, appliedIndex,
                   entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
                   proofPhases, proofAcks, phaseAt, voterEpochAt,
                   snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

StartElection ==
    /\ logicalClock >= electionDeadline
    /\ term < MaxTerm
    /\ term' = term + 1
    /\ electionDeadline' = logicalClock + ElectionTimeout
    /\ votes' = {}
    /\ leaderElected' = FALSE
    /\ electionProofPhase' = NoPhase
    /\ electionProofVotes' = {}
    /\ acknowledgements' =
          [i \in LogSlots |-> IF i > commitIndex THEN {} ELSE acknowledgements[i]]
    /\ UNCHANGED <<phase, voterEpoch, logicalClock,
                   lastIndex, commitIndex, appliedIndex,
                   entryKinds, entryTerms, entryVoterEpochs,
                   proofPhases, proofAcks, phaseAt, voterEpochAt,
                   snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

GrantVote ==
    /\ ~leaderElected
    /\ \E node \in Nodes \ votes:
          votes' = votes \cup {node}
    /\ UNCHANGED <<phase, term, voterEpoch,
                   logicalClock, electionDeadline,
                   leaderElected, electionProofPhase, electionProofVotes,
                   lastIndex, commitIndex, appliedIndex,
                   entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
                   proofPhases, proofAcks, phaseAt, voterEpochAt,
                   snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

ElectLeader ==
    /\ ~leaderElected
    /\ HasQuorum(phase, votes)
    /\ leaderElected' = TRUE
    /\ electionProofPhase' = phase
    /\ electionProofVotes' = votes
    /\ UNCHANGED <<phase, term, voterEpoch,
                   logicalClock, electionDeadline, votes,
                   lastIndex, commitIndex, appliedIndex,
                   entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
                   proofPhases, proofAcks, phaseAt, voterEpochAt,
                   snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

\* ponytail: one pending entry bounds the first model; #51 may pipeline only
\* after proving the same prefix, epoch, and quorum invariants.
Append(kind) ==
    LET index == lastIndex + 1
    IN  /\ leaderElected
        /\ lastIndex = commitIndex
        /\ lastIndex < MaxLogIndex
        /\ kind \in AllowedKinds(phase)
        /\ IF IsConfigurationChange(kind) THEN voterEpoch < MaxTerm ELSE TRUE
        /\ lastIndex' = index
        /\ entryKinds' = [entryKinds EXCEPT ![index] = kind]
        /\ entryTerms' = [entryTerms EXCEPT ![index] = term]
        /\ entryVoterEpochs' = [entryVoterEpochs EXCEPT ![index] = voterEpoch]
        /\ acknowledgements' = [acknowledgements EXCEPT ![index] = {}]
        /\ UNCHANGED <<phase, term, voterEpoch,
                       logicalClock, electionDeadline,
                       votes, leaderElected, electionProofPhase, electionProofVotes,
                       commitIndex, appliedIndex,
                       proofPhases, proofAcks, phaseAt, voterEpochAt,
                       snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

Acknowledge ==
    LET index == commitIndex + 1
    IN  /\ leaderElected
        /\ commitIndex < lastIndex
        /\ \E node \in Nodes \ acknowledgements[index]:
              acknowledgements' =
                  [acknowledgements EXCEPT ![index] = @ \cup {node}]
        /\ UNCHANGED <<phase, term, voterEpoch,
                       logicalClock, electionDeadline,
                       votes, leaderElected, electionProofPhase, electionProofVotes,
                       lastIndex, commitIndex, appliedIndex,
                       entryKinds, entryTerms, entryVoterEpochs,
                       proofPhases, proofAcks, phaseAt, voterEpochAt,
                       snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

Commit ==
    LET index == commitIndex + 1
        kind == entryKinds[index]
        nextPhase == PhaseAfter(phase, kind)
        nextVoterEpoch == VoterEpochAfter(voterEpoch, kind)
        changed == IsConfigurationChange(kind)
    IN  /\ leaderElected
        /\ commitIndex < lastIndex
        /\ HasQuorum(phase, acknowledgements[index])
        /\ commitIndex' = index
        /\ phase' = nextPhase
        /\ voterEpoch' = nextVoterEpoch
        /\ proofPhases' = [proofPhases EXCEPT ![index] = phase]
        /\ proofAcks' = [proofAcks EXCEPT ![index] = acknowledgements[index]]
        /\ phaseAt' = [phaseAt EXCEPT ![index] = nextPhase]
        /\ voterEpochAt' = [voterEpochAt EXCEPT ![index] = nextVoterEpoch]
        /\ votes' = IF changed THEN {} ELSE votes
        /\ leaderElected' = IF changed THEN FALSE ELSE leaderElected
        /\ electionProofPhase' = IF changed THEN NoPhase ELSE electionProofPhase
        /\ electionProofVotes' = IF changed THEN {} ELSE electionProofVotes
        /\ UNCHANGED <<term, logicalClock, electionDeadline,
                       lastIndex, appliedIndex,
                       entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
                       snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

Apply ==
    /\ appliedIndex < commitIndex
    /\ appliedIndex' = appliedIndex + 1
    /\ UNCHANGED <<phase, term, voterEpoch,
                   logicalClock, electionDeadline,
                   votes, leaderElected, electionProofPhase, electionProofVotes,
                   lastIndex, commitIndex,
                   entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
                   proofPhases, proofAcks, phaseAt, voterEpochAt,
                   snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

TakeSnapshot ==
    /\ snapshotIndex < appliedIndex
    /\ snapshotIndex' = appliedIndex
    /\ snapshotTerm' = entryTerms[appliedIndex]
    /\ snapshotVoterEpoch' = voterEpochAt[appliedIndex]
    /\ snapshotPhase' = phaseAt[appliedIndex]
    /\ UNCHANGED <<phase, term, voterEpoch,
                   logicalClock, electionDeadline,
                   votes, leaderElected, electionProofPhase, electionProofVotes,
                   lastIndex, commitIndex, appliedIndex,
                   entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
                   proofPhases, proofAcks, phaseAt, voterEpochAt>>

CrashRecover ==
    /\ appliedIndex > snapshotIndex
    /\ appliedIndex' = snapshotIndex
    /\ UNCHANGED <<phase, term, voterEpoch,
                   logicalClock, electionDeadline,
                   votes, leaderElected, electionProofPhase, electionProofVotes,
                   lastIndex, commitIndex,
                   entryKinds, entryTerms, entryVoterEpochs, acknowledgements,
                   proofPhases, proofAcks, phaseAt, voterEpochAt,
                   snapshotIndex, snapshotTerm, snapshotVoterEpoch, snapshotPhase>>

Next ==
    \/ Tick
    \/ StartElection
    \/ GrantVote
    \/ ElectLeader
    \/ \E kind \in {Command, BeginJoint, Finalize}: Append(kind)
    \/ Acknowledge
    \/ Commit
    \/ Apply
    \/ TakeSnapshot
    \/ CrashRecover

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ phase \in Phases
    /\ term \in 1..MaxTerm
    /\ voterEpoch \in 1..MaxTerm
    /\ logicalClock \in 0..MaxClock
    /\ electionDeadline \in 0..(MaxClock + ElectionTimeout)
    /\ votes \subseteq Nodes
    /\ leaderElected \in BOOLEAN
    /\ electionProofPhase \in Phases \cup {NoPhase}
    /\ electionProofVotes \subseteq Nodes
    /\ lastIndex \in 0..MaxLogIndex
    /\ commitIndex \in 0..lastIndex
    /\ appliedIndex \in 0..commitIndex
    /\ lastIndex - commitIndex \in 0..1
    /\ entryKinds \in [LogSlots -> EntryKinds]
    /\ entryTerms \in [LogSlots -> 0..MaxTerm]
    /\ entryVoterEpochs \in [LogSlots -> 0..MaxTerm]
    /\ acknowledgements \in [LogSlots -> SUBSET Nodes]
    /\ proofPhases \in [LogSlots -> Phases \cup {NoPhase}]
    /\ proofAcks \in [LogSlots -> SUBSET Nodes]
    /\ phaseAt \in [LogSlots -> Phases \cup {NoPhase}]
    /\ voterEpochAt \in [LogSlots -> 0..MaxTerm]
    /\ snapshotIndex \in 0..commitIndex
    /\ snapshotTerm \in 0..MaxTerm
    /\ snapshotVoterEpoch \in 1..MaxTerm
    /\ snapshotPhase \in Phases

AuthorityHasQuorum ==
    leaderElected =>
        /\ electionProofPhase \in Phases
        /\ HasQuorum(electionProofPhase, electionProofVotes)

QuorumsIntersect ==
    \A left, right \in SUBSET Nodes:
        (HasQuorum(phase, left) /\ HasQuorum(phase, right)) =>
            (left \cap right) # {}

CommittedWithQuorum ==
    \A index \in 1..commitIndex:
        /\ proofPhases[index] \in Phases
        /\ HasQuorum(proofPhases[index], proofAcks[index])

SequentialVoterTransition ==
    /\ phase = IF commitIndex = 0 THEN StableOld ELSE phaseAt[commitIndex]
    /\ voterEpoch = IF commitIndex = 0 THEN 1 ELSE voterEpochAt[commitIndex]
    /\ \A index \in 1..commitIndex:
          LET priorPhase == IF index = 1 THEN StableOld ELSE phaseAt[index - 1]
              priorEpoch == IF index = 1 THEN 1 ELSE voterEpochAt[index - 1]
          IN  /\ entryKinds[index] \in AllowedKinds(priorPhase)
              /\ phaseAt[index] = PhaseAfter(priorPhase, entryKinds[index])
              /\ voterEpochAt[index] =
                    VoterEpochAfter(priorEpoch, entryKinds[index])

EpochsMonotonic ==
    /\ \A index \in 1..commitIndex:
          /\ entryTerms[index] <= term
          /\ entryVoterEpochs[index] <= voterEpochAt[index]
          /\ voterEpochAt[index] <= voterEpoch
    /\ \A left, right \in 1..commitIndex:
          left < right =>
              /\ entryTerms[left] <= entryTerms[right]
              /\ voterEpochAt[left] <= voterEpochAt[right]

SnapshotCoversCommittedOnly ==
    /\ snapshotIndex <= appliedIndex
    /\ snapshotIndex <= commitIndex
    /\ IF snapshotIndex = 0
          THEN /\ snapshotTerm = 0
               /\ snapshotVoterEpoch = 1
               /\ snapshotPhase = StableOld
          ELSE /\ snapshotTerm = entryTerms[snapshotIndex]
               /\ snapshotVoterEpoch = voterEpochAt[snapshotIndex]
               /\ snapshotPhase = phaseAt[snapshotIndex]

====
