# winit 0.30.13 vendor baseline

The source is the published `winit` 0.30.13 crate from crates.io. Its archive
SHA-256 is `a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`,
matching `Cargo.lock`. The crate is Apache-2.0 licensed; its upstream repository
revision is recorded in `vendor/winit/.cargo_vcs_info.json`.

WordCraft temporarily patches crates.io to this exact source so the desktop
platform fixes can remain reviewable and be removed together. This baseline
commit contains no behavioral source changes. Remove the override and vendor
tree when WordCraft's resolved winit release contains every retained fix; keep
the regression guards until that upgrade is verified.
