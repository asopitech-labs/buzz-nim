# Nimino platform release contract

`v1/contract.json` fixes the desktop and WSL artifact matrix owned by issue
#62. macOS arm64, macOS x86_64, Linux x86_64, and the qualified WSL2 x86_64
bundle are the complete v1 matrix. A native Windows installer is deliberately
absent: Windows users run the canonical Ubuntu 24.04 WSL2 bundle.

Every desktop lane must pass its signing preflight before building. The
updater manifest and WSL bundle then bind to the same `nimino.release-set`.
This issue may publish a draft candidate only; issue #63 owns promotion.
