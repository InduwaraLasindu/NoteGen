//! Bounded symbol/count checks. No chemical reading is corrected or invented.
//! Symbols: https://iupac.org/what-we-do/periodic-table-of-elements/
//! Stoichiometry: https://goldbook.iupac.org/terms/view/S06026
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const ELEMENTS: &str = "H He Li Be B C N O F Ne Na Mg Al Si P S Cl Ar K Ca Sc Ti V Cr Mn Fe Co Ni Cu Zn Ga Ge As Se Br Kr Rb Sr Y Zr Nb Mo Tc Ru Rh Pd Ag Cd In Sn Sb Te I Xe Cs Ba La Ce Pr Nd Pm Sm Eu Gd Tb Dy Ho Er Tm Yb Lu Hf Ta W Re Os Ir Pt Au Hg Tl Pb Bi Po At Rn Fr Ra Ac Th Pa U Np Pu Am Cm Bk Cf Es Fm Md No Lr Rf Db Sg Bh Hs Mt Ds Rg Cn Nh Fl Mc Lv Ts Og";
const MAX_COUNT: u64 = 1_000_000_000_000;
pub const LIMITS: &str = "Checks supported element symbols, positive integer indices, nested groups, hydrate components, explicit ionic charges, and element/charge bookkeeping across a reaction. This does not establish chemical plausibility, oxidation state, isotope balance or recognition accuracy. Variables, fractional coefficients and unsupported notation require manual review. Source spelling and typography are preserved.";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Formula {
    pub source: String,
    pub coefficient: u64,
    pub atoms: BTreeMap<String, u64>,
    pub charge: i64,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reaction {
    pub source: String,
    pub reactants: Vec<Formula>,
    pub products: Vec<Formula>,
    pub left_atoms: BTreeMap<String, u64>,
    pub right_atoms: BTreeMap<String, u64>,
    pub left_charge: i64,
    pub right_charge: i64,
    pub balanced: bool,
    pub note: String,
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG400", message).into()
}
fn add(
    target: &mut BTreeMap<String, u64>,
    source: &BTreeMap<String, u64>,
    coefficient: u64,
) -> Result<()> {
    for (element, count) in source {
        let value = count
            .checked_mul(coefficient)
            .and_then(|v| target.get(element).copied().unwrap_or(0).checked_add(v))
            .filter(|v| *v <= MAX_COUNT)
            .ok_or_else(|| fail("Chemical atom counts exceed the supported bound"))?;
        target.insert(element.clone(), value);
    }
    Ok(())
}
fn number(bytes: &[u8], cursor: &mut usize) -> Result<u64> {
    let start = *cursor;
    while bytes.get(*cursor).is_some_and(u8::is_ascii_digit) {
        *cursor += 1;
    }
    if start == *cursor {
        return Ok(1);
    }
    anyhow::ensure!(
        *cursor - start <= 6,
        fail("Chemical count exceeds six digits")
    );
    let n = std::str::from_utf8(&bytes[start..*cursor])?.parse::<u64>()?;
    anyhow::ensure!(
        (1..=1_000_000).contains(&n),
        fail("Chemical indices and coefficients must be positive bounded integers")
    );
    Ok(n)
}
fn group(
    bytes: &[u8],
    cursor: &mut usize,
    closing: Option<u8>,
    depth: usize,
) -> Result<BTreeMap<String, u64>> {
    anyhow::ensure!(depth <= 8, fail("Chemical grouping exceeds eight levels"));
    let mut atoms = BTreeMap::new();
    let mut groups = 0;
    while let Some(&ch) = bytes.get(*cursor) {
        if Some(ch) == closing {
            *cursor += 1;
            anyhow::ensure!(groups > 0, fail("Chemical group is empty"));
            return Ok(atoms);
        }
        let part = if ch == b'(' || ch == b'[' {
            *cursor += 1;
            group(
                bytes,
                cursor,
                Some(if ch == b'(' { b')' } else { b']' }),
                depth + 1,
            )?
        } else if ch.is_ascii_uppercase() {
            let start = *cursor;
            *cursor += 1;
            if bytes.get(*cursor).is_some_and(u8::is_ascii_lowercase) {
                *cursor += 1;
            }
            let symbol = std::str::from_utf8(&bytes[start..*cursor])?;
            anyhow::ensure!(
                ELEMENTS.split_whitespace().any(|known| known == symbol),
                fail("Unknown element symbol or unsupported chemical variable")
            );
            BTreeMap::from([(symbol.to_owned(), 1)])
        } else {
            return Err(fail(
                "Unsupported chemical notation, unmatched group or misplaced index",
            ));
        };
        let count = number(bytes, cursor)?;
        add(&mut atoms, &part, count)?;
        groups += 1;
    }
    anyhow::ensure!(
        closing.is_none() && groups > 0,
        fail("Chemical formula is empty or has an unclosed group")
    );
    Ok(atoms)
}
fn normalized(input: &str) -> Result<String> {
    anyhow::ensure!(
        !input.is_empty() && input.len() <= 4096 && !input.chars().any(char::is_control),
        fail("Chemical notation is empty or exceeds its byte/control-character bound")
    );
    let mut input = input.trim();
    for wrapper in ["\\ce{", "\\mathrm{"] {
        if let Some(body) = input
            .strip_prefix(wrapper)
            .and_then(|s| s.strip_suffix('}'))
        {
            input = body;
            break;
        }
    }
    let mut result = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '₀'..='₉' => result.push(char::from(b'0' + (ch as u32 - '₀' as u32) as u8)),
            '·' | '∙' => result.push('.'),
            '⁰' | '¹' | '²' | '³' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹' | '⁺' | '⁻' =>
            {
                result.push('^');
                result.push('{');
                let mut current = Some(ch);
                while let Some(ch) = current {
                    result.push(match ch {
                        '⁰' => '0',
                        '¹' => '1',
                        '²' => '2',
                        '³' => '3',
                        '⁴' => '4',
                        '⁵' => '5',
                        '⁶' => '6',
                        '⁷' => '7',
                        '⁸' => '8',
                        '⁹' => '9',
                        '⁺' => '+',
                        '⁻' => '-',
                        _ => return Err(fail("Only a trailing superscript charge is supported")),
                    });
                    current = if chars.peek().is_some_and(|c| "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻".contains(*c))
                    {
                        chars.next()
                    } else {
                        None
                    };
                }
                result.push('}');
            }
            '_' => {
                let braced = chars.peek() == Some(&'{');
                if braced {
                    chars.next();
                }
                let mut digits = 0;
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    result.push(chars.next().unwrap());
                    digits += 1;
                }
                anyhow::ensure!(
                    digits > 0 && (!braced || chars.next() == Some('}')),
                    fail("Subscript index is empty or unsupported")
                );
            }
            ch if ch.is_whitespace() => {
                return Err(fail(
                    "Whitespace inside a formula is ambiguous; use a reaction with spaced separators",
                ));
            }
            ch => result.push(ch),
        }
    }
    Ok(result)
}
pub fn formula(input: &str) -> Result<Formula> {
    let mut normalized = normalized(input)?;
    for phase in ["(aq)", "(s)", "(l)", "(g)"] {
        if normalized.ends_with(phase) {
            normalized.truncate(normalized.len() - phase.len());
            break;
        }
    }
    let mut charge = 0i64;
    if let Some(caret) = normalized.rfind('^') {
        let suffix = &normalized[caret + 1..];
        let text = if suffix.starts_with('{') {
            suffix
                .strip_prefix('{')
                .and_then(|text| text.strip_suffix('}'))
                .ok_or_else(|| fail("Ionic charge braces are unbalanced"))?
        } else {
            suffix
        };
        anyhow::ensure!(
            !text.contains(['{', '}']),
            fail("Ionic charge grouping is unsupported")
        );
        anyhow::ensure!(
            !text.is_empty() && text.len() <= 7,
            fail("Chemical charge is empty or oversized")
        );
        let sign = match text.as_bytes().last() {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => {
                return Err(fail(
                    "Superscripts must be an explicit trailing ionic charge; isotopes require manual review",
                ));
            }
        };
        let digits = &text[..text.len() - 1];
        let mut index = 0;
        let magnitude = number(digits.as_bytes(), &mut index)?;
        anyhow::ensure!(
            index == digits.len() && magnitude <= 10_000,
            fail("Unsupported ionic charge")
        );
        charge = sign * magnitude as i64;
        normalized.truncate(caret);
    } else if normalized.ends_with(['+', '-']) {
        let sign = if normalized.ends_with('+') { 1 } else { -1 };
        normalized.pop();
        anyhow::ensure!(
            !normalized.as_bytes().last().is_some_and(u8::is_ascii_digit),
            fail(
                "Charge next to a numeric index is ambiguous; use an explicit superscript/caret charge"
            )
        );
        charge = sign;
    }
    let mut coefficient = 1u64;
    let mut atoms = BTreeMap::new();
    if normalized.ends_with('e')
        && normalized[..normalized.len() - 1]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        let mut cursor = 0;
        coefficient = number(&normalized.as_bytes()[..normalized.len() - 1], &mut cursor)?;
        anyhow::ensure!(
            charge == -1,
            fail("An electron entity requires an explicit unit negative charge")
        );
        return Ok(Formula {
            source: input.into(),
            coefficient,
            atoms,
            charge: -(coefficient as i64),
            note: LIMITS.into(),
        });
    }
    for (index, component) in normalized.split('.').enumerate() {
        let mut cursor = 0;
        let factor = number(component.as_bytes(), &mut cursor)?;
        if index == 0 {
            coefficient = factor;
        }
        let part = group(component.as_bytes(), &mut cursor, None, 0)?;
        anyhow::ensure!(
            cursor == component.len(),
            fail("Chemical formula contains unsupported trailing notation")
        );
        // A leading stoichiometric coefficient applies to the whole hydrated entity.
        add(&mut atoms, &part, if index == 0 { 1 } else { factor })?;
    }
    let unscaled = atoms;
    let mut atoms = BTreeMap::new();
    add(&mut atoms, &unscaled, coefficient)?;
    charge = charge
        .checked_mul(coefficient as i64)
        .ok_or_else(|| fail("Chemical charge count overflow"))?;
    Ok(Formula {
        source: input.into(),
        coefficient,
        atoms,
        charge,
        note: LIMITS.into(),
    })
}
pub fn reaction(input: &str) -> Result<Reaction> {
    anyhow::ensure!(
        !input.is_empty() && input.len() <= 4096,
        fail("Reaction exceeds the supported byte bound")
    );
    let text = input
        .replace("<=>", "->")
        .replace(['→', '⇌', '⇋'], "->")
        .replace("=>", "->");
    let sides = text.split("->").collect::<Vec<_>>();
    anyhow::ensure!(
        sides.len() == 2,
        fail("Choose exactly one supported reaction arrow")
    );
    let separator = regex::Regex::new(r"\s+\+\s+")?;
    let parse = |side: &str| -> Result<Vec<Formula>> {
        let parts = separator.split(side.trim()).collect::<Vec<_>>();
        anyhow::ensure!(
            !parts.is_empty() && parts.len() <= 64,
            fail("Reaction has too many entities")
        );
        parts.into_iter().map(|part| formula(part.trim())).collect()
    };
    let reactants = parse(sides[0])?;
    let products = parse(sides[1])?;
    let totals = |entities: &[Formula]| -> Result<(BTreeMap<String, u64>, i64)> {
        let mut atoms = BTreeMap::new();
        let mut charge = 0i64;
        for entity in entities {
            add(&mut atoms, &entity.atoms, 1)?;
            charge = charge
                .checked_add(entity.charge)
                .ok_or_else(|| fail("Reaction charge overflow"))?;
        }
        Ok((atoms, charge))
    };
    let (left_atoms, left_charge) = totals(&reactants)?;
    let (right_atoms, right_charge) = totals(&products)?;
    let balanced = left_atoms == right_atoms && left_charge == right_charge;
    Ok(Reaction {
        source: input.into(),
        reactants,
        products,
        left_atoms,
        right_atoms,
        left_charge,
        right_charge,
        balanced,
        note: LIMITS.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_latex_indices_nested_groups_and_hydrates_preserve_original_notation() {
        for source in ["Al₂(SO₄)₃", "Al_2(SO_4)_3", "Al_{2}(SO_{4})_{3}"] {
            let result = formula(source).unwrap();
            assert_eq!(
                result.atoms,
                BTreeMap::from([("Al".into(), 2), ("S".into(), 3), ("O".into(), 12)])
            );
            assert_eq!(result.source, source);
        }
        let hydrate = formula("2CuSO₄·5H₂O").unwrap();
        assert_eq!(hydrate.atoms["H"], 20);
        assert_eq!(hydrate.atoms["O"], 18);
        assert_eq!(hydrate.atoms["Cu"], 2);
        assert_eq!(formula("K₄[Fe(CN)₆]").unwrap().atoms["N"], 6);
        assert_eq!(formula("Og").unwrap().atoms["Og"], 1);
    }
    #[test]
    fn reactions_check_both_element_counts_and_explicit_charge_without_rebalancing() {
        assert!(reaction("2H₂ + O₂ → 2H₂O").unwrap().balanced);
        let source = "H2 + O2 -> H2O";
        let result = reaction(source).unwrap();
        assert!(!result.balanced);
        assert_eq!(result.source, source);
        assert!(reaction("H^+ + OH^- -> H2O").unwrap().balanced);
        assert!(reaction("Fe^{3+} + 3Cl^- -> FeCl3").unwrap().balanced);
        assert!(reaction("Fe^{3+} + e^- -> Fe^{2+}").unwrap().balanced);
        assert_eq!(formula("SO₄²⁻").unwrap().charge, -2);
        assert!(!reaction("Fe^{2+} -> Fe^{3+}").unwrap().balanced);
    }
    #[test]
    fn ambiguous_unsupported_and_hostile_notation_fail_with_bounded_work() {
        for value in [
            "Fe3+", "H0O", "Xx2O", "(H2O", "H_{}O", "^{14}C", "C9999999", "H2 + O2", "RCH3", "",
            "H\nO", "Fe^{3+", "Fe^3+}",
        ] {
            assert!(formula(value).is_err(), "{value}");
        }
        assert!(formula(&"(".repeat(1000)).is_err());
        assert!(formula(&"C".repeat(4097)).is_err());
        assert!(reaction("H2+O2->H2O").is_err());
        assert!(reaction("H2 -> H2 -> H2").is_err());
        assert!(formula("(C999999)999999").is_ok());
        assert!(formula("((C999999)999999)999999").is_err());
    }
}
