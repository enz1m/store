// SPDX license classification and pretty-print.

/// Classification of an SPDX license expression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LicenseKind {
    /// Recognised free / open-source license.
    Foss,
    /// A non-empty string that isn't recognised as FOSS.
    Proprietary,
    /// Empty / missing license.
    #[default]
    Unknown,
}

impl LicenseKind {}

/// Recognised FOSS SPDX prefixes. Complete-license names like `GPL-3.0`,
/// `Apache-2.0`, `MIT`, `BSD-3-Clause` all begin with one of these prefixes.
/// This is a lightweight classifier — not a replacement for a full SPDX parser
/// — but good enough for a store-page trust badge. Kept alphabetised so
/// additions are cheap to audit.
const FOSS_LICENSE_PREFIXES: &[&str] = &[
    "0BSD",
    "AFL",
    "AGPL",
    "APACHE",
    "APSL",
    "ARTISTIC",
    "BOOST",
    "BSD",
    "BSL",
    "CC-BY",
    "CC0",
    "CDDL",
    "ECOS",
    "EFL",
    "EPL",
    "EUPL",
    "FTL",
    "GFDL",
    "GPL",
    "HPND",
    "ICU",
    "IJG",
    "ISC",
    "LGPL",
    "LIBPNG",
    "LICENSEREF-", // Treat custom refs as FOSS by default; noted as "unknown" via UI hint.
    "MIT",
    "MPL",
    "MS-PL",
    "MS-RL",
    "NCSA",
    "OFL",
    "OPENSSL",
    "OSL",
    "POSTGRESQL",
    "PYTHON",
    "RUBY",
    "SISSL",
    "UNLICENSE",
    "UPL",
    "VIM",
    "W3C",
    "WTFPL",
    "X11",
    "XFREE86",
    "ZLIB",
    "ZPL",
];

/// Classify an SPDX license expression. Handles `AND` / `OR` combinations by
/// treating a compound expression as FOSS iff every atom is FOSS — anything
/// with a non-FOSS atom (e.g. `MIT AND LicenseRef-Proprietary`) is flagged
/// Proprietary. An empty string is `Unknown`.
pub(crate) fn classify_license(spdx: &str) -> LicenseKind {
    let trimmed = spdx.trim();
    if trimmed.is_empty() {
        return LicenseKind::Unknown;
    }
    // Split on `AND` / `OR` / `WITH` (case-insensitive) + parens/whitespace.
    let atoms: Vec<&str> = trimmed
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|s| !s.is_empty())
        .filter(|s| {
            let up = s.to_ascii_uppercase();
            up != "AND" && up != "OR" && up != "WITH"
        })
        .collect();
    if atoms.is_empty() {
        return LicenseKind::Unknown;
    }
    let all_foss = atoms.iter().all(|atom| {
        let up = atom.to_ascii_uppercase();
        FOSS_LICENSE_PREFIXES
            .iter()
            .any(|prefix| up.starts_with(prefix))
    });
    if all_foss {
        LicenseKind::Foss
    } else {
        LicenseKind::Proprietary
    }
}

/// Map a raw SPDX expression to a compact human-readable label, mirroring
/// what a person would call the license verbally. Trims common `-only` /
/// `-or-later` suffixes into `+` shorthand, strips the version scaffolding
/// from `GPL-3.0` → `GPL 3.0`, and collapses `LicenseRef-proprietary=...`
/// URL forms to a plain `Proprietary`. Falls back to the input trimmed +
/// whitespace-normalised so an unknown-but-legible SPDX string still reads
/// cleanly in the tile.
pub(crate) fn pretty_license(spdx: &str) -> String {
    let trimmed = spdx.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // `LicenseRef-proprietary`, `LicenseRef-proprietary=https://...` — both
    // collapse to "Proprietary". Flathub uses these for closed-source apps
    // with a link to the vendor's EULA.
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("LICENSEREF-PROPRIETARY") || upper == "PROPRIETARY" {
        return "Proprietary".to_owned();
    }
    // Compound expressions: prettify each atom in place, preserve joiners.
    // Simple tokeniser preserves whitespace between atoms via `split_whitespace`.
    if upper.contains(" AND ") || upper.contains(" OR ") || upper.contains(" WITH ") {
        let out: Vec<String> = trimmed
            .split_whitespace()
            .map(|tok| {
                let up = tok.to_ascii_uppercase();
                if up == "AND" || up == "OR" || up == "WITH" {
                    up.to_ascii_lowercase()
                } else {
                    pretty_single_license(tok)
                }
            })
            .collect();
        return out.join(" ");
    }
    pretty_single_license(trimmed)
}

// Prettify a single (non-compound) SPDX atom. Split out so the compound
// branch above can reuse it per-token.
fn pretty_single_license(atom: &str) -> String {
    // Strip surrounding parens (SPDX allows `(MIT)` etc.).
    let atom = atom.trim_matches(|c: char| c == '(' || c == ')');
    let upper = atom.to_ascii_uppercase();
    // Extract `-or-later` / `-only` suffixes to render as `+` / no marker.
    let (base, marker) = if let Some(stripped) = upper.strip_suffix("-OR-LATER") {
        (stripped, "+")
    } else if let Some(stripped) = upper.strip_suffix("-ONLY") {
        (stripped, "")
    } else {
        (upper.as_str(), "")
    };

    // Well-known families: turn `GPL-3.0` → `GPL 3.0`, `MIT` → `MIT`,
    // `APACHE-2.0` → `Apache 2.0`, `BSD-3-CLAUSE` → `BSD 3-Clause`, etc.
    // Table of (upper-prefix, display-prefix). Keep the longer prefix first
    // so `LGPL-` beats `GPL-`.
    const FAMILIES: &[(&str, &str)] = &[
        ("AGPL-", "AGPL "),
        ("LGPL-", "LGPL "),
        ("GPL-", "GPL "),
        ("APACHE-", "Apache "),
        ("MPL-", "MPL "),
        ("EPL-", "EPL "),
        ("BSL-", "BSL "),
        ("CDDL-", "CDDL "),
        ("EUPL-", "EUPL "),
        ("CC-BY-SA-", "CC BY-SA "),
        ("CC-BY-", "CC BY "),
        ("CC0-", "CC0 "),
        ("OFL-", "OFL "),
        ("ZPL-", "ZPL "),
    ];
    for (family, display) in FAMILIES {
        if let Some(rest) = base.strip_prefix(family) {
            // `rest` is the version part (e.g. `3.0`, `3-CLAUSE`). Lowercase
            // the suffix so `BSD-3-CLAUSE` → `BSD 3-Clause`, preserving
            // hyphens in multi-part suffixes.
            let suffix = format_license_suffix(rest);
            return format!("{display}{suffix}{marker}");
        }
    }
    // BSD family: `BSD-3-CLAUSE` → `BSD 3-Clause`; `BSD-2-CLAUSE` → `BSD 2-Clause`.
    if let Some(rest) = base.strip_prefix("BSD-") {
        return format!("BSD {}{}", format_license_suffix(rest), marker);
    }
    // Short single-word licenses we spell as-is (canonical casing).
    const EXACT: &[(&str, &str)] = &[
        ("MIT", "MIT"),
        ("ISC", "ISC"),
        ("UNLICENSE", "Unlicense"),
        ("0BSD", "0BSD"),
        ("WTFPL", "WTFPL"),
        ("ZLIB", "zlib"),
        ("NCSA", "NCSA"),
        ("CC0", "CC0"),
        ("X11", "X11"),
    ];
    for (up, display) in EXACT {
        if base == *up {
            return format!("{display}{marker}");
        }
    }
    // `LicenseRef-<name>`: strip the prefix and title-case what remains.
    if let Some(rest) = base.strip_prefix("LICENSEREF-") {
        return title_case_word(rest);
    }
    // Fallback: normalise casing on the input but leave content intact.
    // Uppercase the first letter of each hyphen-separated part.
    title_case_word(base)
}

// Lowercase a `-`-suffixed license version fragment so `3-CLAUSE` →
// `3-Clause`, `2.0` → `2.0`, `1.1` → `1.1`. Keeps hyphens intact.
fn format_license_suffix(s: &str) -> String {
    s.split('-')
        .map(title_case_word)
        .collect::<Vec<_>>()
        .join("-")
}

fn title_case_word(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            let head: String = c.to_uppercase().collect();
            let tail: String = chars.as_str().to_ascii_lowercase();
            head + &tail
        }
        None => String::new(),
    }
}

/// Extract a human-readable age rating from an OARS content-rating map.
/// The map values are severity levels (`none`/`mild`/`moderate`/`intense`);
/// we take the highest and map to a familiar "12+" / "17+" style label so a
/// non-specialist reader gets an at-a-glance signal.
pub(crate) fn format_age_rating(rating: &std::collections::HashMap<String, String>) -> String {
    // Ignore the meta "oars-1.0"/"oars-1.1" version key when looking at levels.
    let severity_order = ["intense", "moderate", "mild", "none"];
    let mut max_idx = severity_order.len(); // = "unset"
    for (key, value) in rating {
        if key.starts_with("oars-") {
            continue;
        }
        let v = value.trim().to_ascii_lowercase();
        if let Some(idx) = severity_order.iter().position(|s| *s == v)
            && idx < max_idx
        {
            max_idx = idx;
        }
    }
    match max_idx {
        0 => "17+".to_owned(), // intense
        1 => "12+".to_owned(), // moderate
        2 => "7+".to_owned(),  // mild
        3 => "3+".to_owned(),  // none
        _ => String::new(),    // no keys with recognised severities
    }
}
