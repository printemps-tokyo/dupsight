# Contributing

Thanks for your interest in contributing to dupsight.

## Development

1. Fork / clone the repository
2. Install a stable Rust toolchain (https://rustup.rs)
3. Create a branch: `git switch -c feat/your-change`
4. Make your change and verify locally:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test --all
   cargo build --release
   ```
5. Commit and open a pull request

## Notes

- The tests make their own pictures (`tests/common/mod.rs`), so no image files
  are checked in.
- Changing a hash or the thumbnail changes every distance. Before changing the
  presets or the hashes, measure on real pictures with
  `cargo run --release --example distances -- photos/*.jpg`, and put the
  numbers in the pull request.
- dupsight must never delete, move or write to the files it scans.
