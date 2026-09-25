#!/usr/bin/env python3
"""Run native integration checks in a private PipeWire instance with no hardware modules.
Requires existing pipewire/native development packages; installs nothing. No WirePlumber,
ALSA, V4L2, desktop capture, PulseAudio socket, session DBus, or live-system mutation.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
CONFIG = '''
context.properties = {
    core.daemon = true
    core.name = telorgon-test-media
    support.dbus = false
    default.clock.rate = 48000
    default.clock.quantum = 256
}
context.spa-libs = {
    audio.convert.* = audioconvert/libspa-audioconvert
    support.* = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-metadata }
    { name = libpipewire-module-spa-node-factory }
    { name = libpipewire-module-client-node }
    { name = libpipewire-module-access args = { access.force = unrestricted } }
    { name = libpipewire-module-adapter }
    { name = libpipewire-module-link-factory }
]
context.objects = [
    { factory = spa-node-factory args = {
        factory.name = support.node.driver
        node.name = telorgon.test.driver
        node.group = telorgon.test
        priority.driver = 20000
    } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = telorgon.test.sink
        node.description = "Isolated synthetic sink"
        media.class = Audio/Sink
        audio.position = [ FL FR ]
        object.linger = true
        adapter.auto-port-config = { mode = dsp monitor = true position = preserve }
    } }
]
'''

def main():
    portal_backend_unit = sys.argv[1:] == ["--portal-backend-unit"]
    screen_unit = sys.argv[1:] == ["--screen-unit"]
    hardware_video = sys.argv[1:] == ["--gpu-video"]
    if hardware_video and os.environ.get("TELORGON_TEST_MODE") != "developer-hardware":
        raise SystemExit("--gpu-video requires explicit TELORGON_TEST_MODE=developer-hardware; it uses the GPU, never real capture devices")
    targets = (["video_gpu_hardware"] if hardware_video else sys.argv[1:]) or ["pipewire_isolated", "portal_isolated", "video_isolated"]
    if not hardware_video and not screen_unit and not portal_backend_unit and any(target not in ("pipewire_isolated", "portal_isolated", "video_isolated") for target in targets):
        raise SystemExit("Choose pipewire_isolated, portal_isolated, video_isolated, --screen-unit or --portal-backend-unit")
    with tempfile.TemporaryDirectory(prefix="telorgon-pw-") as directory:
        private = Path(directory)
        config = private / "isolated.conf"
        config.write_text(CONFIG)
        env = os.environ.copy()
        env.update(PIPEWIRE_RUNTIME_DIR=directory, XDG_RUNTIME_DIR=directory,
                   PIPEWIRE_CONFIG_DIR=directory, PIPEWIRE_CONFIG_PREFIX="",
                   TELORGON_TEST_REMOTE="telorgon-test-media", PIPEWIRE_REMOTE="telorgon-test-media")
        # Do not discover user's PipeWire override fragments or autostart services.
        env["XDG_CONFIG_HOME"] = str(private / "config")
        env["XDG_STATE_HOME"] = str(private / "state")
        with (private / "server.log").open("w+") as log:
            server = subprocess.Popen(["pipewire", "-c", config.name], env=env, stdout=log, stderr=log)
            bus_address = f"unix:path={private / 'bus'}"
            bus = subprocess.Popen(["dbus-daemon", "--session", "--nofork", f"--address={bus_address}"],
                                   env=env, stdout=log, stderr=log)
            env["TELORGON_TEST_BUS_ADDRESS"] = bus_address
            try:
                deadline = time.monotonic() + 5
                while not ((private / "telorgon-test-media").exists() and (private / "bus").exists()):
                    if server.poll() is not None or time.monotonic() > deadline:
                        log.seek(0)
                        raise RuntimeError(log.read())
                    time.sleep(0.02)
                # Client must use its normal installed client.conf, never the server config dir.
                client_env = env.copy()
                client_env.pop("PIPEWIRE_CONFIG_DIR", None)
                client_env.pop("PIPEWIRE_CONFIG_PREFIX", None)
                if os.environ.get("TELORGON_PW_DEBUG"):
                    client_env["PIPEWIRE_DEBUG"] = "4"
                subprocess.run(["cargo", "test", "-p", "telorgon", "--no-default-features",
                                "--features", ("shell-screencast-linux" if screen_unit or portal_backend_unit else "video-linux,embedded-vulkan" if hardware_video else
                                               "desktop-audio-linux,audio-linux,midi-linux,portal-client-linux,video-linux"),
                                *(["--lib", "integrations::portals::backend::tests::isolated"] if portal_backend_unit else
                                  ["--lib", "integrations::pipewire::screencast::tests"] if screen_unit else
                                  [arg for target in targets for arg in ("--test", target)]),
                                "--offline", "--", "--ignored", "--nocapture", "--test-threads=1"],
                               cwd=ROOT, env=client_env, check=True)
            finally:
                bus.terminate()
                bus.wait(timeout=5)
                log.flush()
                log.seek(0)
                print(log.read())
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()

if __name__ == "__main__":
    main()
