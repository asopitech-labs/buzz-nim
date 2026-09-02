## Canonical Nimino command, output, and exit policy.

import std/strutils

const
  CliContractName* = "nimino.cli"
  CliContractVersion* = 1
  CliCommandPaths* = [
    "agents.draft-create", "agents.draft-update", "agents.archive",
    "agents.unarchive", "agents.archived", "messages.send",
    "messages.send-diff", "messages.edit", "messages.delete", "messages.get",
    "messages.thread", "messages.search", "messages.vote", "channels.list",
    "channels.get", "channels.search", "channels.create", "channels.update",
    "channels.topic", "channels.purpose", "channels.join", "channels.leave",
    "channels.archive", "channels.unarchive", "channels.delete",
    "channels.members", "channels.add-member", "channels.remove-member",
    "channels.set-add-policy", "canvas.get", "canvas.set", "reactions.add",
    "reactions.remove", "reactions.get", "emoji.list", "emoji.set", "emoji.rm",
    "emoji.export", "emoji.import", "dms.list", "dms.open", "dms.add-member",
    "dms.hide", "users.get", "users.set-profile", "users.presence",
    "users.set-presence", "users.set-status", "workflows.list", "workflows.get",
    "workflows.create", "workflows.update", "workflows.delete",
    "workflows.trigger", "workflows.runs", "workflows.approve", "feed.get",
    "social.publish", "social.set-contacts", "social.event", "social.notes",
    "social.contacts", "social.set-list", "social.list", "notes.set",
    "notes.get", "notes.ls", "notes.rm",
    "repos.create", "repos.get", "repos.list", "repos.bind",
    "repos.protect.list", "repos.protect.set", "repos.protect.remove",
    "projects.create", "projects.get", "projects.list", "projects.add-repo",
    "projects.remove-repo", "projects.update", "projects.delete", "patches.send",
    "patches.get", "patches.list", "patches.status", "issues.create",
    "issues.get", "issues.list", "issues.status", "issues.assign",
    "issues.unassign", "pr.open", "pr.update", "pr.get", "pr.list",
    "pr.status", "media.get", "upload.file", "mem.ls", "mem.get", "mem.hash",
    "mem.set", "mem.patch", "mem.rm", "pack.validate", "pack.inspect",
    "moderation.reports", "moderation.resolve", "moderation.ban",
    "moderation.unban", "moderation.timeout", "moderation.untimeout",
    "moderation.restricted", "moderation.audit",
  ]
  LocalCommandPaths = ["pack.validate", "pack.inspect"]
  ReadCommandPaths = [
    "agents.archived", "messages.get", "messages.thread", "messages.search",
    "channels.list", "channels.get", "channels.search", "channels.members",
    "canvas.get", "reactions.get", "emoji.list", "emoji.export", "dms.list",
    "users.get", "users.presence", "workflows.list",
    "workflows.get", "workflows.runs", "feed.get", "social.event",
    "social.notes", "social.contacts", "social.list",
    "notes.get", "notes.ls", "repos.get", "repos.list", "repos.protect.list",
    "projects.get", "projects.list", "patches.get", "patches.list", "issues.get",
    "issues.list", "pr.get", "pr.list", "media.get", "mem.ls", "mem.get",
    "mem.hash", "moderation.reports", "moderation.restricted",
    "moderation.audit",
  ]

type
  CliIoMode* = enum
    cimLocal
    cimRelayRead
    cimRelayWrite

  CliPolicyTarget* = enum
    cptNone
    cptEvent
    cptCommunity
    cptMembership
    cptDm
    cptWorkflow
    cptModeration

  CliCommandError* = enum
    cceNone
    cceUnknownCommand

  CliCommandDecision* = object
    accepted*: bool
    error*: CliCommandError
    ioMode*: CliIoMode
    requiresAuth*: bool
    outputContract*: string
    policyTarget*: CliPolicyTarget

  CliFailureKind* = enum
    cfkUsage
    cfkRelay
    cfkNetwork
    cfkAuth
    cfkKey
    cfkConflict
    cfkNotFound
    cfkDeliveryUnknown
    cfkOther

  CliFailureDecision* = object
    category*: string
    exitCode*: int
    retryable*: bool

proc hasPath(paths: openArray[string]; path: string): bool =
  # ponytail: linear scan over a fixed 115-command grammar; generate a lookup
  # only if this bounded table becomes measurable.
  for candidate in paths:
    if candidate == path:
      return true
  false

proc targetFor(path: string): CliPolicyTarget =
  if path.startsWith("workflows."):
    if path == "workflows.delete": cptEvent else: cptWorkflow
  elif path.startsWith("agents."):
    cptMembership
  elif path.startsWith("channels."):
    if path in ["channels.join", "channels.leave", "channels.members",
        "channels.add-member", "channels.remove-member",
        "channels.set-add-policy"]:
      cptMembership
    else:
      cptEvent
  elif path.startsWith("dms."):
    cptDm
  elif path.startsWith("moderation."):
    cptModeration
  elif path.startsWith("messages.") or path.startsWith("canvas.") or
      path.startsWith("reactions.") or
      path.startsWith("emoji.") or path.startsWith("social.") or
      path.startsWith("users.") or path.startsWith("feed.") or
      path.startsWith("notes.") or path.startsWith("repos.") or
      path.startsWith("projects.") or path.startsWith("patches.") or
      path.startsWith("issues.") or path.startsWith("pr.") or
      path.startsWith("mem."):
    cptEvent
  else:
    cptNone

proc decideCliCommand*(path: string): CliCommandDecision =
  if not hasPath(CliCommandPaths, path):
    return CliCommandDecision(
      accepted: false,
      error: cceUnknownCommand,
      ioMode: cimLocal,
      requiresAuth: false,
      outputContract: "nimino.cli-output/v1",
      policyTarget: cptNone,
    )
  let local = hasPath(LocalCommandPaths, path)
  CliCommandDecision(
    accepted: true,
    error: cceNone,
    ioMode:
      if local: cimLocal
      elif hasPath(ReadCommandPaths, path): cimRelayRead
      else: cimRelayWrite,
    requiresAuth: not local,
    outputContract: "nimino.cli-output/v1",
    policyTarget: targetFor(path),
  )

proc decideCliFailure*(
    kind: CliFailureKind; status: int; transportRetryable: bool
): CliFailureDecision =
  case kind
  of cfkUsage:
    CliFailureDecision(category: "user_error", exitCode: 1, retryable: false)
  of cfkRelay:
    if status in [401, 403]:
      CliFailureDecision(category: "auth_error", exitCode: 3, retryable: false)
    else:
      CliFailureDecision(
        category: "relay_error",
        exitCode: 2,
        retryable: status in [429, 502, 503, 504],
      )
  of cfkNetwork:
    CliFailureDecision(
      category: "network_error", exitCode: 2, retryable: transportRetryable
    )
  of cfkAuth:
    CliFailureDecision(category: "auth_error", exitCode: 3, retryable: false)
  of cfkKey:
    CliFailureDecision(category: "key_error", exitCode: 3, retryable: false)
  of cfkConflict:
    CliFailureDecision(category: "conflict", exitCode: 5, retryable: false)
  of cfkNotFound:
    CliFailureDecision(category: "not_found", exitCode: 1, retryable: false)
  of cfkDeliveryUnknown:
    CliFailureDecision(
      category: "delivery_unknown", exitCode: 2, retryable: false
    )
  of cfkOther:
    CliFailureDecision(category: "error", exitCode: 4, retryable: false)
