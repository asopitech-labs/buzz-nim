## Pure workflow definition, condition, planning, and run-transition policy.
##
## Adapters decode YAML/JSON, load facts, execute returned effects, and persist
## transitions with compare-and-swap. This module performs no I/O.

import std/[options, strutils, tables]
from std/unicode import runeSubStr

const
  MaxWorkflowExpressionBytes* = 4096
  MaxWorkflowStepIdBytes* = 64
  MaxWorkflowTransitionIdBytes* = 128

type
  WorkflowTriggerKind* = enum
    wtMessagePosted
    wtReactionAdded
    wtDiffPosted
    wtSchedule
    wtWebhook

  WorkflowActionKind* = enum
    waSendMessage
    waSendDm
    waSetChannelTopic
    waAddReaction
    waCallWebhook
    waRequestApproval
    waDelay

  WorkflowTrigger* = object
    kind*: WorkflowTriggerKind
    filter*: string
    emoji*: string
    cron*: string
    interval*: string

  WorkflowAction* = object
    kind*: WorkflowActionKind
    text*: string
    channel*: string
    replyInThread*: bool
    recipient*: string
    topic*: string
    emoji*: string
    url*: string
    httpMethod*: string
    headers*: Table[string, string]
    body*: string
    approver*: string
    message*: string
    timeout*: string
    duration*: string

  WorkflowStep* = object
    id*: string
    name*: string
    condition*: string
    timeoutSecs*: int64
    action*: WorkflowAction

  WorkflowDefinition* = object
    name*: string
    description*: string
    trigger*: WorkflowTrigger
    steps*: seq[WorkflowStep]
    enabled*: bool

  WorkflowValueKind* = enum
    wvEmpty
    wvString
    wvBool
    wvInt
    wvFloat

  WorkflowValue* = object
    case kind*: WorkflowValueKind
    of wvString:
      stringValue*: string
    of wvBool:
      boolValue*: bool
    of wvInt:
      intValue*: int64
    of wvFloat:
      floatValue*: float64
    of wvEmpty:
      discard

  WorkflowPolicyError* = enum
    wpeNone
    wpeNameRequired
    wpeStepsRequired
    wpeInvalidStep
    wpeDuplicateStep
    wpeScheduleMissing
    wpeScheduleConflict
    wpeInvalidSchedule
    wpeReplyRequiresMessage
    wpeInvalidTrigger
    wpeInvalidAction
    wpeDefinitionDisabled
    wpeInvalidCondition
    wpeUnknownVariable
    wpeTypeMismatch
    wpeInvalidTemplate
    wpeRunNotRunning
    wpeInvalidStepIndex
    wpeStaleRevision
    wpeDuplicateTransition
    wpeInvalidTransition
    wpeTerminalState
    wpeInvalidTransitionId

  WorkflowDefinitionDecision* = object
    valid*: bool
    error*: WorkflowPolicyError
    requiresElevatedAuthority*: bool

  WorkflowConditionDecision* = object
    value*: bool
    error*: WorkflowPolicyError

  WorkflowRunStatus* = enum
    wrPending
    wrRunning
    wrWaitingApproval
    wrCompleted
    wrFailed
    wrCancelled

  WorkflowRunState* = object
    status*: WorkflowRunStatus
    currentStep*: int
    revision*: int64

  WorkflowPlanRequest* = object
    definition*: WorkflowDefinition
    state*: WorkflowRunState
    boundChannel*: string
    trigger*: Table[string, WorkflowValue]
    stepOutputs*: Table[string, Table[string, WorkflowValue]]

  WorkflowDirective* = enum
    wdReject
    wdExecuteEffect
    wdSkipStep
    wdCompleteRun

  WorkflowPlanDecision* = object
    directive*: WorkflowDirective
    error*: WorkflowPolicyError
    stepId*: string
    effect*: WorkflowAction

  WorkflowTransitionCommand* = enum
    wcStart
    wcSkipStep
    wcEffectCompleted
    wcAwaitApproval
    wcResume
    wcComplete
    wcFail
    wcCancel

  WorkflowTransitionRequest* = object
    state*: WorkflowRunState
    expectedRevision*: int64
    transitionId*: string
    transitionAlreadyApplied*: bool
    command*: WorkflowTransitionCommand
    stepCount*: int
    stepIndex*: int

  WorkflowPortEffect* = enum
    wpfNone
    wpfPersistTransition

  WorkflowTransitionDecision* = object
    allowed*: bool
    error*: WorkflowPolicyError
    nextState*: WorkflowRunState
    portEffect*: WorkflowPortEffect

  TokenKind = enum
    tkIdentifier
    tkString
    tkNumber
    tkTrue
    tkFalse
    tkLeftParen
    tkRightParen
    tkComma
    tkEqual
    tkNotEqual
    tkGreater
    tkGreaterEqual
    tkLess
    tkLessEqual
    tkAnd
    tkOr
    tkNot
    tkEnd

  Token = object
    kind: TokenKind
    text: string

  ConditionParser = object
    tokens: seq[Token]
    position: int
    values: Table[string, WorkflowValue]
    error: WorkflowPolicyError

proc stringValue*(value: string): WorkflowValue =
  WorkflowValue(kind: wvString, stringValue: value)

proc boolValue*(value: bool): WorkflowValue =
  WorkflowValue(kind: wvBool, boolValue: value)

proc intValue*(value: int64): WorkflowValue =
  WorkflowValue(kind: wvInt, intValue: value)

proc floatValue*(value: float64): WorkflowValue =
  WorkflowValue(kind: wvFloat, floatValue: value)

proc emptyValue*(): WorkflowValue = WorkflowValue(kind: wvEmpty)

proc rejectDefinition(error: WorkflowPolicyError): WorkflowDefinitionDecision =
  WorkflowDefinitionDecision(error: error)

proc parseDurationSeconds(value: string): Option[int64] =
  let duration = value.strip()
  if duration.len == 0:
    return none(int64)
  var multiplier = 1'i64
  var number = duration
  case duration[^1]
  of 'h':
    multiplier = 3600
    number = duration[0 .. ^2]
  of 'm':
    multiplier = 60
    number = duration[0 .. ^2]
  of 's':
    number = duration[0 .. ^2]
  else:
    discard
  try:
    let parsed = parseBiggestInt(number.strip())
    if parsed < 0 or parsed > high(int64) div multiplier:
      return none(int64)
    some(int64(parsed) * multiplier)
  except ValueError:
    none(int64)

proc validCron(expression: string): bool =
  let fields = expression.splitWhitespace()
  if fields.len notin 5 .. 7:
    return false
  for field in fields:
    if field.len == 0:
      return false
    for ch in field:
      if ch notin {'0' .. '9', '*', '/', ',', '-', '?', 'A' .. 'Z', 'a' .. 'z'}:
        return false
  true

proc validStepId(id: string): bool =
  if id.len == 0 or id.len > MaxWorkflowStepIdBytes:
    return false
  for ch in id:
    if not (ch.isAlphaNumeric() or ch == '_') or ch.ord > 127:
      return false
  true

proc validUuid(value: string): bool =
  if value.len != 36:
    return false
  for index, ch in value:
    if index in [8, 13, 18, 23]:
      if ch != '-':
        return false
    elif ch notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc anyPresent(values: openArray[string]): bool =
  for value in values:
    if value.len > 0:
      return true

proc validAction(action: WorkflowAction): bool =
  case action.kind
  of waSendMessage:
    result = not anyPresent([
      action.recipient, action.topic, action.emoji, action.url,
      action.httpMethod, action.body, action.approver, action.message,
      action.timeout, action.duration,
    ]) and action.headers.len == 0
  of waSendDm:
    result = action.recipient.len > 0 and not action.replyInThread and
      not anyPresent([
        action.channel, action.topic, action.emoji, action.url,
        action.httpMethod, action.body, action.approver, action.message,
        action.timeout, action.duration,
      ]) and action.headers.len == 0
  of waSetChannelTopic:
    result = not action.replyInThread and not anyPresent([
      action.text, action.channel, action.recipient, action.emoji, action.url,
      action.httpMethod, action.body, action.approver, action.message,
      action.timeout, action.duration,
    ]) and action.headers.len == 0
  of waAddReaction:
    result = action.emoji.len > 0 and not action.replyInThread and not anyPresent([
      action.text, action.channel, action.recipient, action.topic, action.url,
      action.httpMethod, action.body, action.approver, action.message,
      action.timeout, action.duration,
    ]) and action.headers.len == 0
  of waCallWebhook:
    result = action.url.len > 0 and not action.replyInThread and not anyPresent([
      action.text, action.channel, action.recipient, action.topic,
      action.emoji, action.approver, action.message, action.timeout,
      action.duration,
    ])
  of waRequestApproval:
    result = action.approver.len > 0 and action.message.len > 0 and
      not action.replyInThread and not anyPresent([
        action.text, action.channel, action.recipient, action.topic,
        action.emoji, action.url, action.httpMethod, action.body,
        action.duration,
      ]) and action.headers.len == 0 and
      (action.timeout.len == 0 or parseDurationSeconds(action.timeout).isSome)
  of waDelay:
    result = not action.replyInThread and not anyPresent([
      action.text, action.channel, action.recipient, action.topic,
      action.emoji, action.url, action.httpMethod, action.body,
      action.approver, action.message, action.timeout,
    ]) and action.headers.len == 0 and
      parseDurationSeconds(action.duration).isSome and
      parseDurationSeconds(action.duration).get() <= 270

proc validateWorkflowDefinition*(
    definition: WorkflowDefinition
): WorkflowDefinitionDecision =
  if definition.name.strip().len == 0:
    return rejectDefinition(wpeNameRequired)
  if definition.steps.len == 0:
    return rejectDefinition(wpeStepsRequired)

  var ids = initTable[string, bool]()
  if definition.trigger.filter.len > MaxWorkflowExpressionBytes:
    return rejectDefinition(wpeInvalidCondition)
  case definition.trigger.kind
  of wtMessagePosted, wtDiffPosted:
    if anyPresent([
      definition.trigger.emoji,
      definition.trigger.cron,
      definition.trigger.interval,
    ]):
      return rejectDefinition(wpeInvalidTrigger)
  of wtReactionAdded:
    if anyPresent([definition.trigger.cron, definition.trigger.interval]):
      return rejectDefinition(wpeInvalidTrigger)
  of wtSchedule:
    if anyPresent([definition.trigger.filter, definition.trigger.emoji]):
      return rejectDefinition(wpeInvalidTrigger)
  of wtWebhook:
    if anyPresent([
      definition.trigger.filter,
      definition.trigger.emoji,
      definition.trigger.cron,
      definition.trigger.interval,
    ]):
      return rejectDefinition(wpeInvalidTrigger)

  for step in definition.steps:
    if not validStepId(step.id) or step.timeoutSecs < 0:
      return rejectDefinition(wpeInvalidStep)
    if ids.hasKey(step.id):
      return rejectDefinition(wpeDuplicateStep)
    ids[step.id] = true
    if step.condition.len > MaxWorkflowExpressionBytes:
      return rejectDefinition(wpeInvalidCondition)
    if not validAction(step.action):
      return rejectDefinition(wpeInvalidAction)
    if step.action.kind == waCallWebhook:
      result.requiresElevatedAuthority = true
    if step.action.kind == waSendMessage and step.action.replyInThread and
        definition.trigger.kind notin {wtMessagePosted, wtReactionAdded, wtDiffPosted}:
      return rejectDefinition(wpeReplyRequiresMessage)

  if definition.trigger.kind == wtSchedule:
    let hasCron = definition.trigger.cron.strip().len > 0
    let hasInterval = definition.trigger.interval.strip().len > 0
    if not hasCron and not hasInterval:
      return rejectDefinition(wpeScheduleMissing)
    if hasCron and hasInterval:
      return rejectDefinition(wpeScheduleConflict)
    if hasCron and not validCron(definition.trigger.cron):
      return rejectDefinition(wpeInvalidSchedule)
    if hasInterval:
      let seconds = parseDurationSeconds(definition.trigger.interval)
      if seconds.isNone or seconds.get() < 60:
        return rejectDefinition(wpeInvalidSchedule)
  elif definition.trigger.cron.len > 0 or definition.trigger.interval.len > 0:
    return rejectDefinition(wpeInvalidSchedule)

  result.valid = true
  result.error = wpeNone

proc tokenize(expression: string; error: var WorkflowPolicyError): seq[Token] =
  var index = 0
  while index < expression.len:
    let ch = expression[index]
    if ch.isSpaceAscii():
      inc index
    elif ch.isAlphaAscii() or ch == '_':
      let start = index
      while index < expression.len and
          (expression[index].isAlphaNumeric() or expression[index] == '_'):
        inc index
      let word = expression[start ..< index]
      case word
      of "true": result.add Token(kind: tkTrue, text: word)
      of "false": result.add Token(kind: tkFalse, text: word)
      else: result.add Token(kind: tkIdentifier, text: word)
    elif ch.isDigit() or (ch == '-' and index + 1 < expression.len and
        expression[index + 1].isDigit()):
      let start = index
      inc index
      var dots = 0
      while index < expression.len and
          (expression[index].isDigit() or expression[index] == '.'):
        if expression[index] == '.':
          inc dots
        inc index
      if dots > 1:
        error = wpeInvalidCondition
        return
      result.add Token(kind: tkNumber, text: expression[start ..< index])
    elif ch == '"':
      inc index
      var value = ""
      var closed = false
      while index < expression.len:
        if expression[index] == '"':
          closed = true
          inc index
          break
        if expression[index] == '\\':
          inc index
          if index >= expression.len:
            break
          case expression[index]
          of '"', '\\': value.add expression[index]
          of 'n': value.add '\n'
          of 'r': value.add '\r'
          of 't': value.add '\t'
          else:
            error = wpeInvalidCondition
            return
        else:
          value.add expression[index]
        inc index
      if not closed:
        error = wpeInvalidCondition
        return
      result.add Token(kind: tkString, text: value)
    else:
      template addToken(tokenKind: TokenKind; width: int) =
        result.add Token(
          kind: tokenKind, text: expression[index ..< index + width]
        )
        index += width
      case ch
      of '(' : addToken(tkLeftParen, 1)
      of ')' : addToken(tkRightParen, 1)
      of ',' : addToken(tkComma, 1)
      of '!':
        if index + 1 < expression.len and expression[index + 1] == '=':
          addToken(tkNotEqual, 2)
        else:
          addToken(tkNot, 1)
      of '=':
        if index + 1 < expression.len and expression[index + 1] == '=':
          addToken(tkEqual, 2)
        else:
          error = wpeInvalidCondition
          return
      of '>':
        if index + 1 < expression.len and expression[index + 1] == '=':
          addToken(tkGreaterEqual, 2)
        else:
          addToken(tkGreater, 1)
      of '<':
        if index + 1 < expression.len and expression[index + 1] == '=':
          addToken(tkLessEqual, 2)
        else:
          addToken(tkLess, 1)
      of '&':
        if index + 1 < expression.len and expression[index + 1] == '&':
          addToken(tkAnd, 2)
        else:
          error = wpeInvalidCondition
          return
      of '|':
        if index + 1 < expression.len and expression[index + 1] == '|':
          addToken(tkOr, 2)
        else:
          error = wpeInvalidCondition
          return
      else:
        error = wpeInvalidCondition
        return
  result.add Token(kind: tkEnd)

proc current(parser: ConditionParser): Token = parser.tokens[parser.position]

proc advance(parser: var ConditionParser): Token =
  result = parser.current()
  if parser.position < parser.tokens.high:
    inc parser.position

proc accept(parser: var ConditionParser; kind: TokenKind): bool =
  if parser.current().kind == kind:
    discard parser.advance()
    return true

proc requireBool(parser: var ConditionParser; value: WorkflowValue): bool =
  if value.kind != wvBool and parser.error == wpeNone:
    parser.error = wpeTypeMismatch
  else:
    result = value.boolValue

proc numeric(value: WorkflowValue; number: var float64): bool =
  case value.kind
  of wvInt:
    number = float64(value.intValue)
    true
  of wvFloat:
    number = value.floatValue
    true
  else:
    false

proc compareValues(
    parser: var ConditionParser;
    left, right: WorkflowValue;
    operator: TokenKind
): WorkflowValue =
  var comparison = 0
  var comparable = true
  var leftNumber, rightNumber: float64
  if left.kind == wvInt and right.kind == wvInt:
    comparison = cmp(left.intValue, right.intValue)
  elif numeric(left, leftNumber) and numeric(right, rightNumber):
    comparison = cmp(leftNumber, rightNumber)
  elif left.kind == wvString and right.kind == wvString:
    comparison = cmp(left.stringValue, right.stringValue)
  elif left.kind == wvBool and right.kind == wvBool:
    comparison = cmp(left.boolValue, right.boolValue)
  elif left.kind == wvEmpty and right.kind == wvEmpty:
    comparison = 0
  else:
    comparable = false
  if not comparable:
    if parser.error == wpeNone:
      parser.error = wpeTypeMismatch
    return boolValue(false)
  case operator
  of tkEqual: result = boolValue(comparison == 0)
  of tkNotEqual: result = boolValue(comparison != 0)
  of tkGreater: result = boolValue(comparison > 0)
  of tkGreaterEqual: result = boolValue(comparison >= 0)
  of tkLess: result = boolValue(comparison < 0)
  of tkLessEqual: result = boolValue(comparison <= 0)
  else:
    parser.error = wpeInvalidCondition
    result = boolValue(false)

proc parseOr(parser: var ConditionParser): WorkflowValue

proc parsePrimary(parser: var ConditionParser): WorkflowValue =
  let token = parser.advance()
  case token.kind
  of tkString:
    result = stringValue(token.text)
  of tkTrue:
    result = boolValue(true)
  of tkFalse:
    result = boolValue(false)
  of tkNumber:
    try:
      if '.' in token.text:
        result = floatValue(parseFloat(token.text))
      else:
        result = intValue(int64(parseBiggestInt(token.text)))
    except ValueError:
      parser.error = wpeInvalidCondition
      result = emptyValue()
  of tkLeftParen:
    result = parser.parseOr()
    if not parser.accept(tkRightParen):
      parser.error = wpeInvalidCondition
  of tkIdentifier:
    if not parser.accept(tkLeftParen):
      if not parser.values.hasKey(token.text):
        parser.error = wpeUnknownVariable
        return emptyValue()
      return parser.values[token.text]
    var arguments: seq[WorkflowValue]
    if parser.current().kind != tkRightParen:
      arguments.add parser.parseOr()
      while parser.accept(tkComma):
        arguments.add parser.parseOr()
    if not parser.accept(tkRightParen):
      parser.error = wpeInvalidCondition
      return emptyValue()
    case token.text
    of "str_contains", "str_starts_with", "str_ends_with":
      if arguments.len != 2 or arguments[0].kind != wvString or
          arguments[1].kind != wvString:
        parser.error = wpeTypeMismatch
        return emptyValue()
      let left = arguments[0].stringValue
      let right = arguments[1].stringValue
      case token.text
      of "str_contains": result = boolValue(right in left)
      of "str_starts_with": result = boolValue(left.startsWith(right))
      else: result = boolValue(left.endsWith(right))
    of "str_len":
      if arguments.len != 1 or arguments[0].kind != wvString:
        parser.error = wpeTypeMismatch
        return emptyValue()
      result = intValue(arguments[0].stringValue.len.int64)
    else:
      parser.error = wpeInvalidCondition
      result = emptyValue()
  else:
    parser.error = wpeInvalidCondition
    result = emptyValue()

proc parseUnary(parser: var ConditionParser): WorkflowValue =
  if parser.accept(tkNot):
    return boolValue(not parser.requireBool(parser.parseUnary()))
  result = parser.parsePrimary()

proc parseComparison(parser: var ConditionParser): WorkflowValue =
  result = parser.parseUnary()
  if parser.current().kind in {
    tkEqual, tkNotEqual, tkGreater, tkGreaterEqual, tkLess, tkLessEqual
  }:
    let operator = parser.advance().kind
    result = parser.compareValues(result, parser.parseUnary(), operator)

proc parseAnd(parser: var ConditionParser): WorkflowValue =
  result = parser.parseComparison()
  while parser.accept(tkAnd):
    let left = parser.requireBool(result)
    let right = parser.requireBool(parser.parseComparison())
    result = boolValue(left and right)

proc parseOr(parser: var ConditionParser): WorkflowValue =
  result = parser.parseAnd()
  while parser.accept(tkOr):
    let left = parser.requireBool(result)
    let right = parser.requireBool(parser.parseAnd())
    result = boolValue(left or right)

proc evaluateWorkflowCondition*(
    expression: string;
    values: Table[string, WorkflowValue]
): WorkflowConditionDecision =
  if expression.len == 0:
    return WorkflowConditionDecision(value: true, error: wpeNone)
  if expression.len > MaxWorkflowExpressionBytes:
    return WorkflowConditionDecision(error: wpeInvalidCondition)
  var tokenError = wpeNone
  let tokens = tokenize(expression, tokenError)
  if tokenError != wpeNone:
    return WorkflowConditionDecision(error: tokenError)
  var parser = ConditionParser(tokens: tokens, values: values)
  let value = parser.parseOr()
  if parser.error != wpeNone:
    return WorkflowConditionDecision(error: parser.error)
  if parser.current().kind != tkEnd:
    return WorkflowConditionDecision(error: wpeInvalidCondition)
  if value.kind != wvBool:
    return WorkflowConditionDecision(error: wpeTypeMismatch)
  WorkflowConditionDecision(value: value.boolValue, error: wpeNone)

proc workflowValueText(value: WorkflowValue): string =
  case value.kind
  of wvString: result = value.stringValue
  of wvBool: result = $value.boolValue
  of wvInt: result = $value.intValue
  of wvFloat: result = $value.floatValue
  of wvEmpty: result = ""

proc templateValue(
    path: string;
    trigger: Table[string, WorkflowValue];
    outputs: Table[string, Table[string, WorkflowValue]]
): Option[string] =
  if path.startsWith("trigger."):
    let field = path[8 .. ^1]
    if trigger.hasKey(field):
      return some(workflowValueText(trigger[field]))
  elif path.startsWith("steps."):
    let parts = path.split('.')
    if parts.len == 4 and parts[2] == "output" and
        outputs.hasKey(parts[1]) and outputs[parts[1]].hasKey(parts[3]):
      return some(workflowValueText(outputs[parts[1]][parts[3]]))
  none(string)

proc bech32Polymod(values: openArray[int]): uint32 =
  const generators = [
    0x3b6a57b2'u32,
    0x26508e6d'u32,
    0x1ea119fa'u32,
    0x3d4233dd'u32,
    0x2a1462b3'u32,
  ]
  result = 1
  for value in values:
    let highBits = result shr 25
    result = ((result and 0x1ffffff'u32) shl 5) xor uint32(value)
    for bit in 0 .. 4:
      if ((highBits shr bit) and 1) != 0:
        result = result xor generators[bit]

proc npubEncode(value: string): string =
  if value.len != 64:
    return value
  var bytes: seq[int]
  for index in countup(0, value.high, 2):
    try:
      bytes.add parseHexInt(value[index .. index + 1])
    except ValueError:
      return value

  var data: seq[int]
  var accumulator = 0
  var bits = 0
  for byte in bytes:
    accumulator = (accumulator shl 8) or byte
    bits += 8
    while bits >= 5:
      bits -= 5
      data.add (accumulator shr bits) and 31
    if bits > 0:
      accumulator = accumulator and ((1 shl bits) - 1)
    else:
      accumulator = 0
  if bits > 0:
    data.add (accumulator shl (5 - bits)) and 31

  var checksumInput: seq[int]
  for ch in "npub":
    checksumInput.add ch.ord shr 5
  checksumInput.add 0
  for ch in "npub":
    checksumInput.add ch.ord and 31
  checksumInput.add data
  checksumInput.add @[0, 0, 0, 0, 0, 0]
  let checksum = bech32Polymod(checksumInput) xor 1'u32
  const alphabet = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
  result = "npub1"
  for item in data:
    result.add alphabet[item]
  for shift in countdown(25, 0, 5):
    result.add alphabet[int((checksum shr shift) and 31)]

proc resolveTemplate(
    templateText: string;
    trigger: Table[string, WorkflowValue];
    outputs: Table[string, Table[string, WorkflowValue]];
    error: var WorkflowPolicyError
): string =
  var cursor = 0
  while cursor < templateText.len:
    let relativeStart = templateText[cursor .. ^1].find("{{")
    if relativeStart < 0:
      result.add templateText[cursor .. ^1]
      break
    let start = cursor + relativeStart
    result.add templateText[cursor ..< start]
    let relativeEnd = templateText[start + 2 .. ^1].find("}}")
    if relativeEnd < 0:
      result.add templateText[start .. ^1]
      break
    let finish = start + 2 + relativeEnd
    let expression = templateText[start + 2 ..< finish].strip()
    let parts = expression.split('|', 1)
    let value = templateValue(parts[0].strip(), trigger, outputs)
    if value.isNone:
      result.add templateText[start .. finish + 1]
    else:
      var resolved = value.get()
      if parts.len == 2:
        let filter = parts[1].strip()
        if filter.startsWith("truncate(") and filter.endsWith(")"):
          try:
            let count = parseInt(filter[9 .. ^2].strip())
            if count < 0:
              error = wpeInvalidTemplate
              return
            resolved = resolved.runeSubStr(0, count)
          except ValueError:
            error = wpeInvalidTemplate
            return
        elif filter in ["npub", "truncate_pubkey"]:
          resolved = npubEncode(resolved)
        else:
          error = wpeInvalidTemplate
          return
      result.add resolved
    cursor = finish + 2

proc resolvedAction(
    action: WorkflowAction;
    trigger: Table[string, WorkflowValue];
    outputs: Table[string, Table[string, WorkflowValue]];
    error: var WorkflowPolicyError
): WorkflowAction =
  result = action
  template field(fieldName: untyped) =
    result.fieldName = resolveTemplate(action.fieldName, trigger, outputs, error)
    if error != wpeNone:
      return
  field(text)
  field(channel)
  field(recipient)
  field(topic)
  field(emoji)
  field(url)
  field(body)
  field(approver)
  field(message)
  var headers = initTable[string, string]()
  for key, value in action.headers:
    headers[key] = resolveTemplate(value, trigger, outputs, error)
    if error != wpeNone:
      return
  result.headers = headers

proc planWorkflowStep*(request: WorkflowPlanRequest): WorkflowPlanDecision =
  let definition = validateWorkflowDefinition(request.definition)
  if not definition.valid:
    return WorkflowPlanDecision(error: definition.error)
  if not request.definition.enabled:
    return WorkflowPlanDecision(error: wpeDefinitionDisabled)
  if request.state.status != wrRunning:
    return WorkflowPlanDecision(error: wpeRunNotRunning)
  if request.state.currentStep < 0 or
      request.state.currentStep > request.definition.steps.len:
    return WorkflowPlanDecision(error: wpeInvalidStepIndex)
  if request.state.currentStep == request.definition.steps.len:
    return WorkflowPlanDecision(directive: wdCompleteRun, error: wpeNone)

  let step = request.definition.steps[request.state.currentStep]
  var values = initTable[string, WorkflowValue]()
  for name, value in request.trigger:
    values["trigger_" & name] = value
  for stepId, output in request.stepOutputs:
    for name, value in output:
      values["steps_" & stepId & "_output_" & name] = value
  let condition = evaluateWorkflowCondition(step.condition, values)
  if condition.error != wpeNone:
    return WorkflowPlanDecision(error: condition.error, stepId: step.id)
  if not condition.value:
    return WorkflowPlanDecision(
      directive: wdSkipStep,
      error: wpeNone,
      stepId: step.id,
    )

  var templateError = wpeNone
  let effect = resolvedAction(
    step.action, request.trigger, request.stepOutputs, templateError
  )
  if templateError != wpeNone:
    return WorkflowPlanDecision(error: templateError, stepId: step.id)
  var selectedEffect = effect
  if selectedEffect.kind == waSendMessage:
    var destination = selectedEffect.channel.strip()
    let bound = request.boundChannel.strip()
    if bound.len > 0:
      if destination.len > 0 and destination != bound:
        return WorkflowPlanDecision(error: wpeInvalidAction, stepId: step.id)
      destination = bound
    elif destination.len == 0 and request.trigger.hasKey("channel_id") and
        request.trigger["channel_id"].kind == wvString:
      destination = request.trigger["channel_id"].stringValue.strip()
    if not validUuid(destination):
      return WorkflowPlanDecision(error: wpeInvalidAction, stepId: step.id)
    if selectedEffect.replyInThread and
        (not request.trigger.hasKey("message_id") or
        request.trigger["message_id"].kind != wvString or
        request.trigger["message_id"].stringValue.len == 0):
      return WorkflowPlanDecision(error: wpeInvalidAction, stepId: step.id)
    selectedEffect.channel = destination
  WorkflowPlanDecision(
    directive: wdExecuteEffect,
    error: wpeNone,
    stepId: step.id,
    effect: selectedEffect,
  )

proc rejectedTransition(
    request: WorkflowTransitionRequest;
    error: WorkflowPolicyError
): WorkflowTransitionDecision =
  WorkflowTransitionDecision(error: error, nextState: request.state)

proc decideWorkflowTransition*(
    request: WorkflowTransitionRequest
): WorkflowTransitionDecision =
  if request.transitionId.len == 0 or
      request.transitionId.len > MaxWorkflowTransitionIdBytes:
    return rejectedTransition(request, wpeInvalidTransitionId)
  if request.transitionAlreadyApplied:
    return rejectedTransition(request, wpeDuplicateTransition)
  if request.state.revision != request.expectedRevision:
    return rejectedTransition(request, wpeStaleRevision)
  if request.stepCount < 0 or request.stepIndex < 0 or
      request.stepIndex > request.stepCount:
    return rejectedTransition(request, wpeInvalidStepIndex)
  if request.state.status in {wrCompleted, wrFailed, wrCancelled}:
    return rejectedTransition(request, wpeTerminalState)
  if request.state.revision == high(int64):
    return rejectedTransition(request, wpeInvalidTransition)

  var next = request.state
  case request.command
  of wcStart:
    if request.state.status != wrPending or request.state.currentStep != 0 or
        request.stepIndex != 0 or request.stepCount == 0:
      return rejectedTransition(request, wpeInvalidTransition)
    next.status = wrRunning
  of wcSkipStep, wcEffectCompleted:
    if request.state.status != wrRunning or
        request.state.currentStep != request.stepIndex or
        request.stepIndex >= request.stepCount:
      return rejectedTransition(request, wpeInvalidTransition)
    inc next.currentStep
  of wcAwaitApproval:
    if request.state.status != wrRunning or
        request.state.currentStep != request.stepIndex or
        request.stepIndex >= request.stepCount:
      return rejectedTransition(request, wpeInvalidTransition)
    next.status = wrWaitingApproval
  of wcResume:
    if request.state.status != wrWaitingApproval or
        request.state.currentStep != request.stepIndex or
        request.stepIndex >= request.stepCount:
      return rejectedTransition(request, wpeInvalidTransition)
    next.status = wrRunning
    inc next.currentStep
  of wcComplete:
    if request.state.status != wrRunning or
        request.state.currentStep != request.stepCount or
        request.stepIndex != request.stepCount:
      return rejectedTransition(request, wpeInvalidTransition)
    next.status = wrCompleted
  of wcFail:
    if request.state.currentStep != request.stepIndex:
      return rejectedTransition(request, wpeInvalidTransition)
    next.status = wrFailed
  of wcCancel:
    if request.state.currentStep != request.stepIndex:
      return rejectedTransition(request, wpeInvalidTransition)
    next.status = wrCancelled

  inc next.revision
  WorkflowTransitionDecision(
    allowed: true,
    error: wpeNone,
    nextState: next,
    portEffect: wpfPersistTransition,
  )
