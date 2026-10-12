# Vendored dependencies

This directory contains published third-party crates with narrowly scoped local
patches. Each crate keeps its upstream license and provenance in its own
directory.

| Crate | Version | License | Local patch | Regression evidence |
|---|---|---|---|---|
| winit | 0.30.13 | Apache-2.0 | See per-fix notes under `patches/winit/`; the source baseline is unchanged. | `apps/wordcraft/tests/winit_vendor.rs` |
