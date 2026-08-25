description = "Nim is a statically typed compiled systems programming language."
homepage = "https://nim-lang.org/"
repository = "https://github.com/nim-lang/Nim"
test = "nim --version"

binaries = ["bin/*"]
strip = 1

env = {
  "NIMBLE_DIR": "${HERMIT_ENV}/.hermit/nimble",
  "PATH": "${root}/bin:${PATH}",
}

vars = {
  "release": "2026-04-24-version-2-2-bfeb3146d1638b39f69007a4ae5a23e23ae4e5ef",
}

platform "linux" "amd64" {
  source = "https://github.com/nim-lang/nightlies/releases/download/${release}/nim-${version}-linux_x64.tar.xz"
}

platform "linux" "arm64" {
  source = "https://github.com/nim-lang/nightlies/releases/download/${release}/nim-${version}-linux_arm64.tar.xz"
}

platform "darwin" "amd64" {
  source = "https://github.com/nim-lang/nightlies/releases/download/${release}/nim-${version}-macosx_x64.tar.xz"
}

platform "darwin" "arm64" {
  source = "https://github.com/nim-lang/nightlies/releases/download/${release}/nim-${version}-macosx_arm64.tar.xz"
}

version "2.2.10" {}

sha256sums = {
  "https://github.com/nim-lang/nightlies/releases/download/2026-04-24-version-2-2-bfeb3146d1638b39f69007a4ae5a23e23ae4e5ef/nim-2.2.10-linux_x64.tar.xz": "0a3a38752e97e9d44aa479b3a7b37336dfe0176daf22ee5b5218ad0991ecd211",
  "https://github.com/nim-lang/nightlies/releases/download/2026-04-24-version-2-2-bfeb3146d1638b39f69007a4ae5a23e23ae4e5ef/nim-2.2.10-linux_arm64.tar.xz": "cd86a6e2bcbf029c4870aa51df5c0169345dbf9959889112fd15d403c13ae33a",
  "https://github.com/nim-lang/nightlies/releases/download/2026-04-24-version-2-2-bfeb3146d1638b39f69007a4ae5a23e23ae4e5ef/nim-2.2.10-macosx_x64.tar.xz": "35df59b9bbe9f5dfcdf40a82b41037e6ac499e2ec0be6688cd3dd0e55c8bc851",
  "https://github.com/nim-lang/nightlies/releases/download/2026-04-24-version-2-2-bfeb3146d1638b39f69007a4ae5a23e23ae4e5ef/nim-2.2.10-macosx_arm64.tar.xz": "9a3b012d0680d11d6163dd2f145470b090c1045f5e634f42daf119bea1cb2b5e",
}
