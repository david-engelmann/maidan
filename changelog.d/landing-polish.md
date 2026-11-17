### A lighter landing and a steadier board toolbar

- **Changed:** the landing's recording of the board is an animated AVIF, 158 KB at 1280 px where the GIF was 830 KB at 960 px. A browser that cannot read AVIF still gets the GIF, and the paused or reduced-motion still is an AVIF too (18 KB, where the PNG is 132 KB). `scripts/make-motion.sh` writes both from the same take.
- **Fixed:** the board's toolbar lays out by the board's own width, so a task sheet open beside the board counts. A middling board puts the filters on a row of their own instead of dropping one filter alone onto a second line. A narrow board keeps the phone's single row that scrolls sideways.
- **Fixed:** a tab strip that scrolls sideways (Tools, Activity's filters) fades the edge with more behind it, instead of cutting a tab off mid-word.
- **Fixed:** on a phone the sign-in card's padding left the Cloudflare check too little room for its normal size. It now shows at full width instead of as a square.
