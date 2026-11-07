# Screenshot capture

`landing.rs` is an xtask fixture that renders every landing-page reference screenshot from isolated sample projects (no private data). Use the captures to keep the page colors and layout the same as the app.

1. Copy it to `crates/xtask/src/native/landing.rs`.
2. In `crates/xtask/src/native/mod.rs` add `mod landing;`. In `crates/xtask/src/native/dispatch.rs` (`run_case`) add `"landing" => super::landing::run(&opts)?,`.
3. Run on a clean checkout (no WIP):
   `cargo build --workspace --bins --examples --features terminator/test-support --locked && cargo xtask gui landing --scale 2`
4. Resize `target/validation/native/landing/*.png` to 2000px wide into `public/assets/` (`sips --resampleWidth 2000`).
5. Remove the copy and the two registration lines from xtask.

The window is 1440x900, so measure CSS pixels as capture pixels / 2.
