// The console's CSS rules (UI-003, UI-004). Spacing and type come from the
// tokens in src/styles/00-tokens.css, and media queries use only the named
// breakpoints listed there. `npm run lint` runs this beside eslint, in CI too.
const SPACING = "/^(padding|margin|gap|row-gap|column-gap|scroll-padding|scroll-margin)/";

export default {
  reportNeedlessDisables: true,
  reportInvalidScopeDisables: true,
  rules: {
    // A raw px in a padding, margin or gap: use a --space-* token. A 1, 3, 5,
    // 7 or 9px nudge on a chip or a key cap, and a negative pull, are allowed.
    "declaration-property-value-disallowed-list": {
      [SPACING]: ["/(?<![\\w.-])(?:[2468]|[1-9]\\d+)px/"],
      "font-size": ["/\\d(px|pt)\\b/"],
    },
    // The breakpoints: narrow 380, phone 600, wide 900, xwide 1440.
    "media-feature-name-value-allowed-list": {
      "min-width": ["600px", "900px", "1440px"],
      "max-width": ["380px", "599px", "899px"],
    },
    "media-feature-name-disallowed-list": ["width", "device-width", "min-device-width", "max-device-width"],
    // Mistakes rather than style.
    "at-rule-no-unknown": true,
    "block-no-empty": true,
    "color-no-invalid-hex": true,
    "declaration-block-no-duplicate-custom-properties": true,
    "font-family-no-duplicate-names": true,
    "function-calc-no-unspaced-operator": true,
    "keyframe-declaration-no-important": true,
    "media-feature-name-no-unknown": true,
    "no-duplicate-at-import-rules": true,
    "no-invalid-double-slash-comments": true,
    "no-invalid-position-at-import-rule": true,
    "property-no-unknown": true,
    "selector-pseudo-class-no-unknown": true,
    "selector-pseudo-element-no-unknown": true,
    "selector-type-no-unknown": [true, { ignore: ["custom-elements"] }],
    "string-no-newline": true,
    "unit-no-unknown": true,
  },
};
