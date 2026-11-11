# Contributing

Pull requests are welcome, from a typo to a new module.

1. Read [docs/architecture.md](docs/architecture.md): it says where every fact comes from and
   how to add a module.
2. Before opening a PR, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and
   `cargo test --release`, then run `cargo run --release -- -a --timings` and check the output.
3. On Haiku, build natively: `scripts/haiku-build.sh <ssh-host>` copies the tree over and builds
   it there.

## Ground rules

- **Speed is a feature.** Check `--timings` before and after. Anything that waits (a program, the
  network, a framework load) goes in a `slow` module, and anything slow and stable is cached.
- **No new dependencies** without a very good reason; a few lines of FFI usually beat a crate.
- **No panics.** `unwrap` and `expect` are denied outside tests. A probe that can't answer returns
  `None` and its line is hidden.
- **Parsing is pure and tested.** Keep file and syscall reads thin and put the logic in a function
  that takes text or bytes, so `tests/` can feed it made-up input.
- **Comments say why**: the platform quirk, the cost that was measured, the fallback's reason.
- American spelling in code, docs and output.
