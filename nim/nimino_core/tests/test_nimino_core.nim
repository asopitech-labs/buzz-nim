import std/unittest

import nimino_core

suite "nimino_core package":
  test "the core module is importable":
    check NiminoCoreVersion == "0.1.0"
