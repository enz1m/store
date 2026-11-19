// OARS content-rating breakdown.

// ─── OARS per-category breakdown ─────────────────────────────────────────────

/// One row in the OARS content-rating breakdown dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OarsCategoryRow {
    /// Display group heading: "Violence", "Drugs & Alcohol", etc.
    pub group: String,
    /// The raw OARS key, e.g. "violence-cartoon".
    pub key: String,
    /// Human-readable sub-label, e.g. "Cartoon violence".
    pub label: String,
    /// Severity: "None" / "Mild" / "Moderate" / "Intense".
    pub severity: String,
    /// Short human description of the severity level for this key.
    pub description: String,
}

// Map an OARS key to its human-readable display group, sub-label, and a
// per-severity description. Returns `None` for unrecognised keys (including
// meta keys like `oars-1.1`).
fn oars_key_meta(key: &str) -> Option<(&'static str, &'static str, [&'static str; 4])> {
    // Each entry: (group, sub-label, [none, mild, moderate, intense])
    Some(match key {
        // ── Violence ──
        "violence-cartoon" => (
            "Violence",
            "Cartoon violence",
            [
                "No cartoon violence",
                "Mild cartoon violence",
                "Moderate cartoon violence",
                "Intense cartoon violence",
            ],
        ),
        "violence-fantasy" => (
            "Violence",
            "Fantasy violence",
            [
                "No fantasy violence",
                "Mild fantasy violence",
                "Moderate fantasy violence",
                "Intense fantasy violence",
            ],
        ),
        "violence-realistic" => (
            "Violence",
            "Realistic violence",
            [
                "No realistic violence",
                "Mild realistic violence",
                "Moderate realistic violence",
                "Intense realistic violence",
            ],
        ),
        "violence-bloodshed" => (
            "Violence",
            "Bloodshed",
            [
                "No bloodshed",
                "Mild bloodshed",
                "Moderate bloodshed",
                "Intense bloodshed",
            ],
        ),
        "violence-sexual" => (
            "Violence",
            "Sexual violence",
            [
                "No sexual violence",
                "Mild sexual violence",
                "Moderate sexual violence",
                "Intense sexual violence",
            ],
        ),
        "violence-desecration" => (
            "Violence",
            "Desecration",
            [
                "No desecration of human remains",
                "Mild desecration of human remains",
                "Moderate desecration of human remains",
                "Intense desecration of human remains",
            ],
        ),
        "violence-slavery" => (
            "Violence",
            "Slavery",
            [
                "No depictions of slavery",
                "Depictions of slavery in a historical context",
                "Depictions of slavery outside a historical context",
                "Graphic depictions of slavery",
            ],
        ),
        "violence-worship" => (
            "Violence",
            "Worship",
            [
                "No violence involving places of worship",
                "Mild violence involving places of worship",
                "Moderate violence involving places of worship",
                "Intense violence involving places of worship",
            ],
        ),
        // ── Drugs & Alcohol ──
        "drugs-alcohol" => (
            "Drugs & Alcohol",
            "Alcohol",
            [
                "No references to alcohol",
                "References to alcohol use",
                "Use of alcohol",
                "Promotion of alcohol use",
            ],
        ),
        "drugs-narcotics" => (
            "Drugs & Alcohol",
            "Narcotics",
            [
                "No references to illicit drugs",
                "References to illicit drug use",
                "Use of illicit drugs",
                "Promotion of illicit drug use",
            ],
        ),
        "drugs-tobacco" => (
            "Drugs & Alcohol",
            "Tobacco",
            [
                "No references to tobacco",
                "References to tobacco use",
                "Use of tobacco",
                "Promotion of tobacco use",
            ],
        ),
        // ── Sex & Nudity ──
        "sex-nudity" => (
            "Sex & Nudity",
            "Nudity",
            [
                "No nudity",
                "Brief nudity",
                "Moderate nudity",
                "Graphic nudity",
            ],
        ),
        "sex-themes" => (
            "Sex & Nudity",
            "Sexual themes",
            [
                "No sexual themes",
                "Mild sexual themes",
                "Moderate sexual themes",
                "Intense sexual themes",
            ],
        ),
        "sex-homosexuality" => (
            "Sex & Nudity",
            "Homosexuality",
            [
                "No references to homosexuality",
                "Mild references to homosexuality",
                "Moderate references to homosexuality",
                "Intense references to homosexuality",
            ],
        ),
        "sex-prostitution" => (
            "Sex & Nudity",
            "Prostitution",
            [
                "No references to prostitution",
                "Mild references to prostitution",
                "Moderate references to prostitution",
                "Intense references to prostitution",
            ],
        ),
        "sex-adultery" => (
            "Sex & Nudity",
            "Adultery",
            [
                "No references to adultery",
                "Mild references to adultery",
                "Moderate references to adultery",
                "Intense references to adultery",
            ],
        ),
        "sex-appearance" => (
            "Sex & Nudity",
            "Sexualised appearance",
            [
                "No sexualised characters",
                "Mildly sexualised characters",
                "Moderately sexualised characters",
                "Intensely sexualised characters",
            ],
        ),
        // ── Language ──
        "language-profanity" => (
            "Language",
            "Profanity",
            [
                "No profanity",
                "Mild profanity",
                "Moderate profanity",
                "Intense profanity",
            ],
        ),
        "language-humor" => (
            "Language",
            "Crude humour",
            [
                "No crude humour",
                "Mild crude humour",
                "Moderate crude humour",
                "Intense crude humour",
            ],
        ),
        "language-discrimination" => (
            "Language",
            "Discrimination",
            [
                "No discriminatory language",
                "Mild discriminatory language",
                "Moderate discriminatory language",
                "Intense discriminatory language",
            ],
        ),
        // ── Social ──
        "social-chat" => (
            "Social",
            "Chat",
            [
                "No chat functionality",
                "Text chat with known contacts",
                "Text chat with strangers",
                "Unmoderated text chat with strangers",
            ],
        ),
        "social-info" => (
            "Social",
            "Personal information",
            [
                "No sharing of personal information",
                "Sharing first name or nickname only",
                "Sharing personal details",
                "Sharing detailed personal information",
            ],
        ),
        "social-audio" => (
            "Social",
            "Audio chat",
            [
                "No voice chat",
                "Voice chat with known contacts",
                "Voice chat with strangers",
                "Unmoderated voice chat with strangers",
            ],
        ),
        "social-location" => (
            "Social",
            "Location sharing",
            [
                "No location sharing",
                "Location sharing with known contacts",
                "Location sharing with strangers",
                "Precise location sharing with strangers",
            ],
        ),
        "social-contacts" => (
            "Social",
            "Contacts access",
            [
                "No access to contacts",
                "Contacts access limited to names",
                "Contacts access including details",
                "Full contacts access",
            ],
        ),
        // ── Money ──
        "money-purchasing" => (
            "Money",
            "In-app purchases",
            [
                "No in-app purchases",
                "Optional in-app purchases",
                "Required in-app purchases",
                "Aggressive in-app purchase tactics",
            ],
        ),
        "money-gambling" => (
            "Money",
            "Gambling",
            [
                "No gambling",
                "Simulated gambling",
                "Real gambling with no real money",
                "Real gambling with real money",
            ],
        ),
        "money-advertising" => (
            "Money",
            "Advertising",
            [
                "No advertising",
                "Non-targeted advertising",
                "Targeted advertising",
                "Invasive advertising",
            ],
        ),
        _ => return None,
    })
}

// Severity index: 0 = none, 1 = mild, 2 = moderate, 3 = intense.
fn severity_index(value: &str) -> usize {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => 0,
        "mild" => 1,
        "moderate" => 2,
        "intense" => 3,
        _ => 0,
    }
}

// Human-readable severity label with proper casing.
fn severity_label(idx: usize) -> &'static str {
    match idx {
        0 => "None",
        1 => "Mild",
        2 => "Moderate",
        3 => "Intense",
        _ => "None",
    }
}

/// Build a per-category breakdown of the OARS content-rating map.
///
/// Groups categories under human-readable headings (Violence, Drugs & Alcohol,
/// Language, Sex & Nudity, Social, Money). Each row includes the sub-label,
/// severity, and a human-readable description. Rows are sorted by group then
/// by key for deterministic order. Only recognised OARS keys are returned;
/// the meta version key (`oars-1.0` / `oars-1.1`) is skipped.
///
/// When a recognised key is absent from the map, it is omitted from the
/// result (consistent "absent = omit" policy).
pub(crate) fn categorize_oars(
    rating: &std::collections::HashMap<String, String>,
) -> Vec<OarsCategoryRow> {
    // Deterministic ordering: sort keys alphabetically so we can group.
    let mut keys: Vec<&String> = rating.keys().collect();
    keys.sort();

    let mut rows = Vec::new();
    for key in keys {
        if key.starts_with("oars-") {
            continue;
        }
        if let Some((group, label, descriptions)) = oars_key_meta(key) {
            let sev_idx = severity_index(&rating[key]);
            rows.push(OarsCategoryRow {
                group: group.to_owned(),
                key: key.clone(),
                label: label.to_owned(),
                severity: severity_label(sev_idx).to_owned(),
                description: descriptions[sev_idx].to_owned(),
            });
        }
    }

    // Sort by group for logical clustering, then by key within group.
    // The group ordering follows the canonical OARS order:
    // Violence → Drugs & Alcohol → Sex & Nudity → Language → Social → Money
    let group_order = |g: &str| -> usize {
        match g {
            "Violence" => 0,
            "Drugs & Alcohol" => 1,
            "Sex & Nudity" => 2,
            "Language" => 3,
            "Social" => 4,
            "Money" => 5,
            _ => 6,
        }
    };
    rows.sort_by(|a, b| {
        group_order(&a.group)
            .cmp(&group_order(&b.group))
            .then_with(|| a.key.cmp(&b.key))
    });
    rows
}
