# Vendored dependencies

This directory contains published third-party crates with narrowly scoped local
patches. Each crate keeps its upstream license and provenance in its own
directory.

| Crate | Version | License | Local patch | Regression evidence |
|---|---|---|---|---|
| winit | 0.30.13 | Apache-2.0 | Ten narrowly scoped platform fixes; see `patches/winit/01` through `10` and the exact file/hash map in `patches/winit/source-delta.json`. | `apps/wordcraft/tests/winit_*.rs` |
