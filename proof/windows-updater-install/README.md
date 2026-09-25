# Native Windows updater install proof

Copy this harness to `proof/windows-updater-install` on a separate proof branch of
`steipete/plugins-workspace`. The branch's
`.github/workflows/windows-updater-cleanup-proof.yml` owns the two Windows jobs
described below. The product dependency remains
pinned to candidate `63e8185eb184cf0af6b665f631e2314c7b14d9c6`, not the proof commit.

The test uses public `UpdaterExt` → `updater_builder` → `check` → `Update::install`.
A loopback server supplies synthetic update metadata. No download, signing keys,
WinAPI mocks, production seams, installable package, or Gateway is involved.
Tauri's `MockRuntime` avoids a real webview while retaining its actual resource table
and `cleanup_before_exit` implementation.

The failed-launch case sends an MSI-format marker to the real install entry point,
with only the disposable child process's `SYSTEMROOT` directed to a missing fixture
directory. Actual `ShellExecuteW` must fail and return an I/O error. Baseline cleanup
destroys an owned resource before that return; the candidate must keep it alive.

The NSIS success case launches this package's benign PE executable. It has an
`asInvoker` manifest, records its real PID/arguments, and uses named Windows events
to coordinate a bounded native-process join. The helper must receive `/UPDATE`.
Release and joining precede receipt assertions; runtime errors also signal release
and attempt to join any helper whose PID was recovered.

The MSI success case launches the system `msiexec` in quiet mode with the original
`SYSTEMROOT` and no proof-only arguments. Its entire payload is an eight-byte format
marker, so there is no installable MSI package. This case covers native launcher
acceptance only; it does not prove completed MSI installation, record a helper PID,
or join the native launcher. The disposable runner lifecycle owns any native
launcher cleanup.

Both success cases must invoke real application cleanup exactly once, drop its
resource, and exit with code 0 without returning from `Update::install`. Cases run
concurrently with separate subprocesses, directories, and event names. No completed
MSI or NSIS package installation is claimed.

The PowerShell runner binds the original and patched SHA, requires a fresh evidence
directory and unchanged source/lockfile, and accepts baseline failure only when the
specific early-cleanup result and both success controls are present. Candidate must
pass all three cases. Evidence consists only of synthetic receipts and build output.

The separate OpenClaw job tests the prospective consumer tree. It verifies base
`8416a588dc2a4f006b509df1cba437ffafd41cc8`, checks the SHA256 of
`openclaw-candidate.patch`, applies the patch to the index, and requires
`git write-tree` to produce `b0696779b924dfa4bac2b34c4da5e29c2307fb3b` before running:

```text
cargo test --locked --manifest-path apps/linux/src-tauri/Cargo.toml updater::tests
```

The exact patch SHA256 is
`661c64ae1ae012d523d0d4908cea52594c5af769807560933d07dcae265e58ae`.
This job exercises OpenClaw's companion integration against the pinned updater;
it complements the baseline/candidate native installation-boundary proof.

Run on native Windows with Git, Rust 1.98.1, the MSVC linker/Windows SDK, and network
access to the locked public Cargo dependencies. GitHub `windows-2025` supplies the
native toolchain prerequisites. No Windows execution has been claimed from macOS.
