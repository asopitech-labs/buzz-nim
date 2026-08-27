import std/[tables, unittest]

import nimino_core/domain/workflow_policy

const ChannelId = "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001"

proc basicDefinition(condition = ""): WorkflowDefinition =
  WorkflowDefinition(
    name: "Incident alert",
    trigger: WorkflowTrigger(kind: wtMessagePosted),
    steps: @[
      WorkflowStep(
        id: "notify",
        condition: condition,
        action: WorkflowAction(
          kind: waSendMessage,
          text: "P1: {{trigger.text}}",
          channel: ChannelId,
        ),
      )
    ],
    enabled: true,
  )

proc actionFor(kind: WorkflowActionKind): WorkflowAction =
  result.kind = kind
  case kind
  of waSendMessage:
    result.text = "message"
    result.channel = ChannelId
  of waSendDm:
    result.recipient = "alice"
    result.text = "message"
  of waSetChannelTopic: result.topic = "topic"
  of waAddReaction: result.emoji = "eyes"
  of waCallWebhook: result.url = "https://example.com/hook"
  of waRequestApproval:
    result.approver = "alice"
    result.message = "approve?"
  of waDelay: result.duration = "1s"

suite "Nimino workflow policy":
  test "definition validation owns workflow invariants":
    check validateWorkflowDefinition(basicDefinition()).valid

    var duplicate = basicDefinition()
    duplicate.steps.add duplicate.steps[0]
    check validateWorkflowDefinition(duplicate).error == wpeDuplicateStep

    var schedule = basicDefinition()
    schedule.trigger = WorkflowTrigger(kind: wtSchedule)
    check validateWorkflowDefinition(schedule).error == wpeScheduleMissing

    var polluted = basicDefinition()
    polluted.steps[0].action.url = "https://example.com"
    check validateWorkflowDefinition(polluted).error == wpeInvalidAction

  test "condition evaluation supports the existing expression surface":
    var values = initTable[string, WorkflowValue]()
    values["trigger_text"] = stringValue("P1 incident")
    values["trigger_is_reply"] = boolValue(false)
    values["steps_gate_output_count"] = intValue(3)

    check evaluateWorkflowCondition(
      "str_contains(trigger_text, \"P1\") && trigger_is_reply == false",
      values,
    ).value
    check evaluateWorkflowCondition(
      "steps_gate_output_count >= 3 && str_len(trigger_text) > 5",
      values,
    ).value
    values["large"] = intValue(9_007_199_254_740_993'i64)
    check evaluateWorkflowCondition(
      "large == 9007199254740993", values
    ).value
    check evaluateWorkflowCondition("missing == true", values).error ==
      wpeUnknownVariable

  test "planning returns a resolved effect through the port boundary":
    var trigger = initTable[string, WorkflowValue]()
    trigger["text"] = stringValue("incident")
    trigger["author"] = stringValue("alice")
    let plan = planWorkflowStep(WorkflowPlanRequest(
      definition: basicDefinition("str_contains(trigger_text, \"incident\")"),
      state: WorkflowRunState(status: wrRunning, currentStep: 0, revision: 1),
      trigger: trigger,
      stepOutputs: initTable[string, Table[string, WorkflowValue]](),
    ))
    check plan.directive == wdExecuteEffect
    check plan.effect.kind == waSendMessage
    check plan.effect.text == "P1: incident"

    var crossChannel = basicDefinition()
    check planWorkflowStep(WorkflowPlanRequest(
      definition: crossChannel,
      state: WorkflowRunState(status: wrRunning),
      boundChannel: "018f5e5a-9b7d-7c01-a7bb-46fbe46d0002",
      trigger: trigger,
      stepOutputs: initTable[string, Table[string, WorkflowValue]](),
    )).error == wpeInvalidAction

  test "planning preserves every effect kind and resolves npub formatting":
    for kind in WorkflowActionKind:
      var definition = basicDefinition()
      definition.steps[0].action = actionFor(kind)
      let plan = planWorkflowStep(WorkflowPlanRequest(
        definition: definition,
        state: WorkflowRunState(status: wrRunning, currentStep: 0, revision: 0),
        trigger: initTable[string, WorkflowValue](),
        stepOutputs: initTable[string, Table[string, WorkflowValue]](),
      ))
      check plan.directive == wdExecuteEffect
      check plan.effect.kind == kind

    var definition = basicDefinition()
    definition.steps[0].action.text = "{{trigger.author | npub}}"
    var trigger = initTable[string, WorkflowValue]()
    trigger["author"] = stringValue(
      "e17e5abf7b1dbd363f0ed6fbda2455609727b2555428dea251388c542cd2f03f"
    )
    let encoded = planWorkflowStep(WorkflowPlanRequest(
      definition: definition,
      state: WorkflowRunState(status: wrRunning, currentStep: 0, revision: 0),
      trigger: trigger,
      stepOutputs: initTable[string, Table[string, WorkflowValue]](),
    ))
    check encoded.effect.text ==
      "npub1u9l940mmrk7nv0cw6maa5fz4vztj0vj42s5dagj38zx9gtxj7qls94fpux"

    definition.steps[0].action.text = "{{trigger.text | truncate(2)}}"
    trigger.clear()
    trigger["text"] = stringValue("日本語")
    let truncated = planWorkflowStep(WorkflowPlanRequest(
      definition: definition,
      state: WorkflowRunState(status: wrRunning),
      trigger: trigger,
      stepOutputs: initTable[string, Table[string, WorkflowValue]](),
    ))
    check truncated.effect.text == "日本"

    definition.enabled = false
    check planWorkflowStep(WorkflowPlanRequest(
      definition: definition,
      state: WorkflowRunState(status: wrRunning),
      trigger: trigger,
      stepOutputs: initTable[string, Table[string, WorkflowValue]](),
    )).error == wpeDefinitionDisabled

  test "state transitions are monotonic and reject duplicate work":
    let state = WorkflowRunState(status: wrRunning, currentStep: 0, revision: 4)
    let advanced = decideWorkflowTransition(WorkflowTransitionRequest(
      state: state,
      expectedRevision: 4,
      transitionId: "step-0-complete",
      transitionAlreadyApplied: false,
      command: wcEffectCompleted,
      stepCount: 1,
      stepIndex: 0,
    ))
    check advanced.allowed
    check advanced.nextState == WorkflowRunState(
      status: wrRunning,
      currentStep: 1,
      revision: 5,
    )

    var duplicate = WorkflowTransitionRequest(
      state: advanced.nextState,
      expectedRevision: 4,
      transitionId: "step-0-complete",
      transitionAlreadyApplied: true,
      command: wcEffectCompleted,
      stepCount: 1,
      stepIndex: 0,
    )
    check decideWorkflowTransition(duplicate).error == wpeDuplicateTransition

  test "terminal states cannot transition":
    for terminal in [wrCompleted, wrFailed, wrCancelled]:
      let decision = decideWorkflowTransition(WorkflowTransitionRequest(
        state: WorkflowRunState(status: terminal, currentStep: 1, revision: 2),
        expectedRevision: 2,
        transitionId: "again",
        transitionAlreadyApplied: false,
        command: wcCancel,
        stepCount: 1,
        stepIndex: 1,
      ))
      check decision.error == wpeTerminalState

  test "every legal transition increments exactly one revision":
    let requests = @[
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrPending), expectedRevision: 0, transitionId: "start", command: wcStart, stepCount: 1, stepIndex: 0),
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrRunning, revision: 1), expectedRevision: 1, transitionId: "skip", command: wcSkipStep, stepCount: 1, stepIndex: 0),
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrRunning, revision: 2), expectedRevision: 2, transitionId: "effect", command: wcEffectCompleted, stepCount: 1, stepIndex: 0),
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrRunning, revision: 3), expectedRevision: 3, transitionId: "await", command: wcAwaitApproval, stepCount: 1, stepIndex: 0),
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrWaitingApproval, revision: 4), expectedRevision: 4, transitionId: "resume", command: wcResume, stepCount: 1, stepIndex: 0),
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrRunning, currentStep: 1, revision: 5), expectedRevision: 5, transitionId: "complete", command: wcComplete, stepCount: 1, stepIndex: 1),
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrRunning, revision: 6), expectedRevision: 6, transitionId: "fail", command: wcFail, stepCount: 1, stepIndex: 0),
      WorkflowTransitionRequest(state: WorkflowRunState(status: wrWaitingApproval, revision: 7), expectedRevision: 7, transitionId: "cancel", command: wcCancel, stepCount: 1, stepIndex: 0),
    ]
    for request in requests:
      let decision = decideWorkflowTransition(request)
      check decision.allowed
      check decision.nextState.revision == request.state.revision + 1
      check decision.portEffect == wpfPersistTransition

      var stale = request
      stale.expectedRevision = request.state.revision + 1
      let staleDecision = decideWorkflowTransition(stale)
      check not staleDecision.allowed
      check staleDecision.error == wpeStaleRevision
      check staleDecision.nextState == request.state
