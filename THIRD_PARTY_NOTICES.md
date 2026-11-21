# Third-party notices

The Linux release bundles these programs next to `wraith`:

- **Xray-core** v26.11.9 — Mozilla Public License 2.0 (`LICENSE-Xray.txt` in the release archive). Source: https://github.com/XTLS/Xray-core/tree/v26.11.9
- **geoip.dat / geosite.dat** — from the v2fly / Loyalsoldier projects.

Rust dependencies (egui/eframe, serde, ureq, rustls, ksni, arboard, chacha20poly1305 and others) are listed with their licenses in `Cargo.lock` and on crates.io. The built-in icon glyphs come from egui's bundled fonts (emoji-icon-font, MIT; Noto Emoji, OFL).

## flag-icons

Server flags come from flag-icons v7.6.0 by Panayiotis Lipiridis, MIT License ([assets/LICENSE-flag-icons.txt](assets/LICENSE-flag-icons.txt)). The SVGs are packed into `assets/flags.png`; Nordic-cross flags are cropped from the 4x3 set so the cross stays left of centre. Upstream: https://github.com/lipis/flag-icons.
