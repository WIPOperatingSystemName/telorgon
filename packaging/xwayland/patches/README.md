# Private payload patches

These patches apply with zero fuzz to the locked Xwayland 24.1.13 and xkbcomp
1.5.0 sources. They are Linux/glibc-specific and are not general upstream patches.
Original source copyright/license notices remain intact. Telorgon's added code
uses the repository's GPL-3.0-or-later license; review corresponding-source and
notice obligations before redistribution. A successful build is not that review.

`0001` replaces Xwayland's shell-mediated keymap invocation with `posix_spawn`,
a bounded sealed memfd input, literal argv, private includes, a five-second
timeout and child reaping. Data memfds use `MFD_NOEXEC_SEAL` when supported;
older kernels rejecting that flag use a sealed data memfd that is never executed.
It requires an absolute `TELORGON_XKBCOMP` and an owned
0700 `TELORGON_XKB_OUTPUT_DIR`, plus the existing absolute `-xkbdir` argument.
There is no `/tmp` fallback. The helper's stdin receives the sealed keymap input,
stdout is `/dev/null`, stderr remains the server's diagnostic stream, and all FDs
above 2 are closed by spawn file actions. The input limit is 4 MiB.

Both patches arm `PR_SET_PDEATHSIG(SIGKILL)` and check the parent again after
arming. Launchers should supply `TELORGON_XWAYLAND_PARENT_PID` for the server;
Xwayland supplies `TELORGON_XKBCOMP_PARENT_PID` for its compiler. Mismatched
identities fail before helper operation. Without an explicit expected parent,
the observed non-init parent is used and checked again after arming.

The exact patched keymap functions are compiled into the helper-test harness.
Five layouts/variants/options, path relocation, parent mismatch, input limit,
timeout/reaping and scratch symlink rejection have local test coverage. Actual
server parent-death/crash cycles, runtime FD inventory and real-session keymap
replacement still require integration qualification.
