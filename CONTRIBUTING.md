# Contributing to brp

Thanks for contributing to brp. This project is a native, peer-to-peer screen
sharing application written in Rust.

## Before you start

- Search existing issues and pull requests before opening a new one.
- For a bug, use the bug-report template. For a proposal, use the feature-request
  template so maintainers can discuss the direction before significant work starts.
- Please report security vulnerabilities privately; see [SECURITY.md](SECURITY.md).

## Development setup

The Linux prerequisites, Windows setup, and the project architecture are in the
[README](README.md). Build and run the hardware-independent test suite with:

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Some capture and codec paths require a supported desktop, GPU, or installed
FFmpeg libraries. Do not make a pull request depend on hardware-only validation;
add focused tests or fakes where possible and state any hardware validation you
performed.

## Making a change

1. Create a focused branch from the current default branch.
2. Keep the change small and include tests when behavior changes.
3. Format and lint the workspace, then run the most relevant tests (or the full
   workspace suite when practical).
4. Update the README or other documentation when users, contributors, or
   operators need to know about the change.
5. Open a pull request using the provided template.

Avoid unrelated formatting changes. Preserve the bounded-queue behavior in the
media pipeline: when a stage falls behind, frames are intentionally dropped to
keep latency bounded.

## Pull requests

Describe the problem, the solution, and how you tested it. Call out platform
coverage—especially when a change affects Linux, Windows, PipeWire, FFmpeg, or
GPU-specific code. A maintainer may ask for a narrower scope, tests, or changes
before merging.

By contributing, you agree that your contributions are licensed under the
project's [MIT License](LICENSE).
