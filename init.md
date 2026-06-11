# Quick local install

From this repository:

```bash
cargo build --release
./install.sh --no-build
hash -r
which ac
ac --help
```

Initialize a project and build the local agent image:

```bash
ac
```

Try a contained shell or agent:

```bash
ac shell
ac codex
ac opencode
```

For untrusted prompts, avoid copying saved credentials:

```bash
ac codex --no-login
```
