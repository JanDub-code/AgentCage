# Quick local install

On the Linux x86_64 source-build host described in [README.md](README.md#requirements), from this repository:

```bash
./install.sh &&
export PATH="$HOME/.local/bin:$PATH" &&
hash -r &&
command -v ac &&
ac --help
```

The installer builds with `--locked` for `x86_64-unknown-linux-gnu`. To install an existing build, use `./install.sh --no-build`; the source binary is expected at `target/x86_64-unknown-linux-gnu/release/ac` or the corresponding `CARGO_TARGET_DIR` location.

Leave the installer checkout, choose the actual project, and initialize the local agent image:

```bash
cd /path/to/project &&
ac &&
ac shell
```

Replace the project path. Initialization needs rootless Podman/crun/systemd cgroups v2 and network downloads.

Run one agent, with or without saved login sync:

```bash
ac codex
ac codex --no-login
```

Agent options go after `--`, for example `ac codex -- --help`. The project stays writable and networking stays enabled in either mode. Container removal preserves project changes, the image, and saved login/environment state. See [SECURITY.md](SECURITY.md).
