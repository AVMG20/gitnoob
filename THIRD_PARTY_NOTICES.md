# Third-party notices

gitnoob is GPL-3.0-only. What is below ships inside the app bundles and keeps its own licence.

## magequery

- Version: 0.24.1 (pinned in `scripts/magequery.mjs`)
- Source: https://github.com/cresset-tools/magequery (release tag `magequery-v0.24.1`)
- Licence: European Union Public Licence 1.2 (EUPL-1.2), full text in [licenses/magequery-EUPL-1.2.txt](licenses/magequery-EUPL-1.2.txt)

gitnoob runs it as a sidecar for the "Magento wiring" view of a class. The macOS, Linux and Windows binaries are the project's own release builds, checked against their published sha256. The Intel half of the universal macOS binary is built from the same release's source, since the release has no Intel Mac build.

## Inter

- The typeface the interface is set in, bundled through `@fontsource-variable/inter`
- Source: https://github.com/rsms/inter
- Copyright 2016 The Inter Project Authors
- Licence: SIL Open Font License 1.1 (OFL-1.1), full text in [licenses/Inter-OFL-1.1.txt](licenses/Inter-OFL-1.1.txt)

The npm packages and Rust crates compiled into the app carry their own licences (MIT, Apache-2.0, ISC and similar); their notices are in each package's source.
