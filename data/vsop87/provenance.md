# VSOP87E series files

The nine `VSOP87E.*` files are the complete VSOP87 solution of Bretagnon & Francou (1988), variant E
(barycentric rectangular coordinates, dynamical ecliptic and equinox J2000), for the Sun and the eight planets,
unmodified as retrieved 2026-10-16 from https://ftp.imcce.fr/pub/ephem/planets/vsop87/ (also mirrored at
https://cdsarc.cds.unistra.fr/ftp/VI/81/). Each record is one term `a·cos(b + c·t)`; the last three columns are
its amplitude, phase and frequency, and the header above a series names the coordinate and the power of `t`.

| File | Terms | SHA-256 |
|---|---:|---|
| VSOP87E.sun | 6634 | `ef9fe6239d8c2c9b451dfd8dfa5fbaf1427003cf0d58e995a3c36c4e590cfbbc` |
| VSOP87E.mer | 7880 | `edc57deaaaab9a44f08de65446ec059d10f8317d54c8985165068cd4e2f0c69e` |
| VSOP87E.ven | 4657 | `f50c1adb4bf1fe0bb3282cdccabaa1998e2b6e0f64e2aa347300db87ae3ede95` |
| VSOP87E.ear | 5556 | `d0781ee55a8215787c5d3dfc020a4c9f2ca61c1dbd8f7d67d5a96e6f7b47b5cb` |
| VSOP87E.mar | 7575 | `77977d409aa3a88c24321a47634dbd55a9afdc799a179977e7ab1919339da36e` |
| VSOP87E.jup | 4542 | `cad69c1c50a9e0e37192cd8de2cc03d91f29fd895b57650e226326d6bda0af65` |
| VSOP87E.sat | 7522 | `e32bee854f545af10f97291a4fe185fdd0ce1bd2727b92b1455f0cd5e610b70e` |
| VSOP87E.ura | 5125 | `993d97d76b69a84207402c4a9a2c78a65efbb406f80bfdcc2467357a5e1a8f12` |
| VSOP87E.nep | 2389 | `031d196efcd59347f2d770bb04aab59d57e494aee4750c6f6d9dc377311ed29b` |

`build.rs` packs them at compile time into `vsop87e.bin` in Cargo's `OUT_DIR` (51,880 terms, 1,245,776 bytes,
SHA-256 `13bfa39001d6981b2bc055120fee8cc3aaec89186add49be1ba3776751933de5`), which `formulas/constants.rs` embeds
as `VSOP87E_TERMS`. The values are the published decimals parsed as IEEE doubles, in file order; nothing is
truncated. The layout is documented in `build.rs`.

Credit: P. Bretagnon and G. Francou, "Planetary theories in rectangular and spherical variables. VSOP87
solutions", Astronomy & Astrophysics 202, 309 (1988); files distributed by IMCCE, Paris Observatory.
