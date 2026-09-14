# Terminator branding

The selected logo is **Red Eye**: terminal + AI, with a red machine sensor
as a subtle nod to The Terminator. Generated with the built-in image tool.

`red-eye-source.png` is the checked-in, unmasked source artwork.
Shipped icons (`terminator.png`, `terminator-512.png`, `terminator.icns`) apply
Apple's continuous-corner mask so Dock/Finder match other macOS app icons.
The GUI embeds `terminator.png` as its window icon (also the running Dock tile).
`terminator.icns` is copied into the application bundle by `cargo xtask package`.

Regenerate from the source artwork:

```sh
swift crates/app/assets/branding/generate-icons.swift
```

The Linux archive includes the 512 px derivative as `terminator.png`; when installing the included
desktop entry, install this image as `terminator.png` in the user's icon
directory (`~/.local/share/icons/hicolor/512x512/apps/`), or set the desktop
entry's `Icon` value to the installed image's absolute path.
