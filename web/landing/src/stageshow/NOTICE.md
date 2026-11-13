# Notices

`kit/` is a subset of the Stageshow kit by Benji Taylor, MIT licensed (see
`LICENSE`), copied from
[github.com/benjitaylor/skills](https://github.com/benjitaylor/skills) at
commit `299140fa`, path `src/stageshow/kit/`.

Copied unchanged: the stage (`stage.tsx`, `act.ts`, `clock.ts`, `cursor.tsx`,
`cursors.tsx`, `ease.ts`, `keys.tsx`, `looks.ts`, `measure.ts`,
`reduced-motion.ts`, `util.ts` and their CSS), `window.tsx`, `motion/Roll`,
`motion/Collapse`, `controls/Button`, `controls/Segmented`, `parts/Avatar`
and `parts/BrandMark`.

Changed: `index.ts` exports only those parts, and `brands/index.ts` lists only
the GitHub, Slack and Linear marks, imported `?inline` so the build never emits
them as separate files. Every other vendored file is as upstream wrote it, so `eslint.config.js` skips `kit/`; the scenes beside it are the
landing's own code and are linted.

The brand artwork in `kit/brands/` is not covered by the Stageshow license.
Its sources, hashes and terms are in `kit/brands/SOURCES.md`, with SVGL's
license in `kit/brands/LICENSE-SVGL.txt`. The marks remain with their owners.
