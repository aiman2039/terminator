# Screenshot capture

`landing.rs` is an xtask fixture that renders every landing-page screenshot from isolated sample projects (no private data).

1. Copy it to `crates/xtask/src/native/landing.rs`.
2. In `crates/xtask/src/native.rs` add `mod landing;` and the case `"landing" => landing::run(&opts)?,`.
3. Run on a clean checkout (no WIP):
   `cargo build --workspace --bins --examples --features terminator/test-support --locked && cargo xtask gui landing --scale 2`
4. Resize `target/validation/native/landing/*.png` to 2000px wide into `public/assets/`.
