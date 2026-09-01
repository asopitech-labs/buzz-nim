#!/bin/sh
set -eu

python3 /tests/verify.py \
  --evidence /logs/artifacts/nimino-evidence.json \
  --skill-file /home/nimino/.claude/skills/context-health-check/SKILL.md \
  --reward /logs/verifier/reward.json \
  --details /logs/verifier/details.json
