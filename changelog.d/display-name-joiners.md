### Display names keep the joiners real names need

- **Fixed:** a display name may now carry ZWNJ/ZWJ between two letters of the same joining script (Persian, Devanagari and other Indic names), ZWJ inside an emoji sequence, and VS16 right after an emoji. Leading, trailing, doubled or cross-script joiners, joiners between Latin letters, and a lone VS16 are still refused (#1576).
