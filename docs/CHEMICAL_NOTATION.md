# Chemical notation checks

Use **Projects & Tools → Recovery → Chemical notation checks**, or:

```text
notegen-cli tools validate-notation formula "Al₂(SO₄)₃"
notegen-cli tools validate-notation reaction "2H₂ + O₂ → 2H₂O"
```

The checker preserves the supplied text. It reports atom counts and explicit
ionic charges, including nested groups and hydrate components. Reactions report
whether both sides have matching supported counts and charges; they never
rebalance a transcription. Spaces around reaction `+` separators distinguish
entities from ionic charge signs. Use an explicit charge such as `Fe^{3+}` or
`SO₄²⁻`; ambiguous forms remain for review.

The grammar accepts positive integer coefficients and indices, up to eight nested
groups, 64 entities per reaction side and 4,096 UTF-8 bytes. Variables, fractional
coefficients, isotope notation and unsupported syntax require manual review.
Passing these checks establishes bookkeeping only, not chemical plausibility or
recognition accuracy. Compare technical symbols and typography with the scan.
Existing editable equation export can retain confirmed subscripts/superscripts;
these checks do not rewrite document blocks.

Element symbols follow the [IUPAC periodic table](https://iupac.org/what-we-do/periodic-table-of-elements/).
The reaction bookkeeping follows the [IUPAC stoichiometry definition](https://goldbook.iupac.org/terms/view/S06026).
