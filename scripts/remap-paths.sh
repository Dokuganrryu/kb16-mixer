# Sourced by the build scripts. Prints a `cargo --config` value that keeps build-machine paths
# (user names among them) out of release binaries: panic locations embed source paths.
# $1 = the rustflags config key, $2 = the repository root.
# rustc sees native paths, so they are converted where cygpath exists (Git Bash on Windows).
remap_config() {
  native() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else echo "$1"; fi; }
  cargo_dir=$(native "${CARGO_HOME:-$HOME/.cargo}")
  root_dir=$(native "$2")
  echo "$1=['--remap-path-prefix=$cargo_dir=/cargo', '--remap-path-prefix=$root_dir=/kb16-mixer']"
}
