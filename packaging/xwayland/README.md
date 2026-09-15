# Xwayland payload preparation

This directory implements offline compilation, private-library staging, archive
packing, source-byte verification and build preflight. The pinned helpers have
built successfully in an isolated Ubuntu 24.04/glibc 2.39 environment. Their
relocated keymap compiler has helper-test coverage. **This is not a qualified
X11 desktop distribution.** Runtime tracing, supervised server launch, complete
source/license review, reproducibility and clean-image qualification remain open.

Run prerequisite checks without downloading or installing anything:

```sh
python3 packaging/xwayland/build.py --preflight
python3 packaging/xwayland/build.py --source-cache /absolute/verified-source-cache
python3 -m unittest discover -s packaging/xwayland -p 'test_*.py'
```

The source verifier compares archive bytes with the six pinned hashes. It does
not establish signature provenance or recheck upstream advisories. Additional
build inputs and libraries must be locked before redistribution.

In a prepared isolated x86-64/glibc 2.39 build root, with this directory mounted
read-only at `/recipe` and downloaded archives at `/inputs`, compile into a fresh
work directory:

```sh
python3 /recipe/build.py --source-cache /inputs --compile /work/build-run
python3 /recipe/stage.py --work /work/build-run --stage /work/build-run/stage
python3 /recipe/test_helpers.py --work /work/build-run --stage /work/build-run/stage
python3 /recipe/pack.py --stage /work/build-run/stage \
  --components /work/build-run/components.json --output /work/xwayland.payload
```

The tested environment used Canonical's signed Ubuntu Base 24.04.4 archive,
recorded in `sources.lock.toml`, plus the binary package inputs recorded in
`build-debs.lock.json`. `build-packages.lock.tsv` records the resulting installed
inventory. Package installation occurred solely in a private build root under
`/tmp`, with service startup disabled. Compilation used a network-disabled
Bubblewrap namespace. No workstation packages were changed. Environment bootstrap
automation and independent bit-for-bit reproduction remain outstanding; the input
locks alone are not proof of reproducibility.

The recipe builds X.Org protocol headers 2024.1 because Ubuntu's bundled headers
lack Present 1.4, then Wayland protocols 1.47, libXfont2 2.0.8, xkbcomp 1.5.0,
keyboard data 2.46 and Xwayland 24.1.13. It applies the version-specific private
helper patches with zero fuzz. See [patch scope](patches/README.md).

Staging resolves DT_NEEDED recursively through an explicit allowlist, records
per-file dependencies and glibc symbol requirements, and replaces helper/library
RUNPATHs with `$ORIGIN/../lib` and `$ORIGIN`. It does not execute `ldd` or bundle
graphics drivers. Runtime dlopen tracing remains necessary. The packer accepts:

```text
bin/Xwayland
bin/xkbcomp
lib/<private library files>
share/X11/xkb/<keyboard data files>
licenses/<notices and source references>
```

`libwayland-client.so.0` is a **host runtime dependency**, alongside EGL/GL/GBM. Host Mesa's EGL
vendor uses this same SONAME in Xwayland's process. Bundling the build root's older Wayland client
can shadow the host copy and prevent Mesa from loading: the September 13 capture exposed
`undefined symbol: wl_fixes_interface` with host Mesa 26.0.8. The staged helper can link while
Glamor silently falls back to software, so DT_NEEDED closure alone is insufficient evidence.
The deployment host must supply a Wayland client ABI compatible with both the helper and its
graphics stack. The staging inventory now includes the runtime-policy hash and its text; the
packer rejects files that shadow declared host runtime SONAMEs, including stale older stages.
No driver is bundled, forced, or substituted by this change.

After staging, run this loader-only check on each intended deployment host (Mesa example):

```sh
python3 packaging/xwayland/check_egl_linkage.py --stage /absolute/stage \
  --vendor-library libEGL_mesa.so.0
```

This loads the helper's direct ELF dependencies with its staged library choices, then links the
selected EGL vendor in a fresh process with eager symbol resolution. It does not execute Xwayland,
connect to a display, open a DRM device or initialize EGL. The old stage fails on the reported host;
the corrected stage passes. This is a specific ABI compatibility check, not GPU qualification,
an exact simulation of every dynamic-loader environment, or a check of every vendor. Real Glamor
initialization and DMA-BUF commits must still be checked with the latency harness/smoke test.

Staging must be quiescent and private to the build owner. Symlinks, hardlinks and
special files are rejected; private SONAME aliases must be ordinary files. A
components JSON array records each component's `name`, `version`,
`input_sha256`, and `license`. Inputs may be source archives, patches, or the
original distribution library bytes; the field does not claim all inputs are
source archives. It is a build inventory, not proof of license compliance.

```sh
python3 packaging/xwayland/pack.py \
  --stage /absolute/stage \
  --components /absolute/components.json \
  --output /absolute/xwayland.payload
```

Archive version 1 has an eight-byte `TLXWP001` signature, a four-byte little-endian
JSON length, canonical JSON, then independent raw DEFLATE streams in entry order.
Offsets are relative to the compressed-data section. Entries record path, mode,
compressed size, extracted size and SHA-256. The manifest records target
`x86_64-unknown-linux-gnu`, proposed glibc floor `2.39`, and Xwayland `24.1.13`.
Bounds are 64 MiB for the entire archive, 256 MiB extracted, 16,384 files and
4 MiB of manifest JSON. Packing is deterministic for identical inputs with the
same Python/zlib implementation; a reproducible release must pin that toolchain.

The optional `desktop-xwayland-embedded` Cargo feature requires the absolute
`TELORGON_XWAYLAND_PAYLOAD` build input. Cargo validates all entries and embeds
the validated bytes in a downstream executable when its code uses
`telorgon::xwayland::payload::embedded()`. There is no network build step, PATH
search or fallback. Missing payloads fail the build. Managed desktop integration
starts the embedded helper asynchronously when compatibility is enabled; the
integrated path still requires live validation. See
[the smoke test](../../docs/X11_SMOKE_TEST.md).

Runtime `payload::extract()` must run on a preparation worker. Its explicit cache
root must be absolute, owned by the current user and mode 0700. Ancestors are
walked without following symlinks and checked for unsafe write permissions.
Cache roots can contain spaces or shell metacharacters; no shell is invoked.
Files are verified before use, modes are 0400/0500, completed generations are
published atomically, and the returned lease holds a shared usage lock. Corrupt
generations fail closed. Failed preparations can leave a private `.preparing-*`
directory; automated garbage collection is not implemented. Do not delete any
generation used by a running helper.

Extraction integrity is not application isolation from the same Unix user. The
host loader/graphics stack and executable cache policy still need real-server
and clean-image validation. Static symbol-floor checks and relocated compiler
tests cannot substitute for them. No synthetic
unit-test fixture may be redistributed as a compatibility payload.
