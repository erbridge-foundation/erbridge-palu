//! Line-by-line JSONL parsing of the two SDE files into domain types.

use std::io::BufRead;

use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;

use crate::model::{RawHull, RawHullCatalog, SecClass, System};

use super::types::{
    ATTR_JUMP_DRIVE_RANGE, ATTR_JUMP_DRIVE_RANGE_BONUS, JDC_SKILL_TYPE_ID, RawStargate, RawSystem,
    RawType, RawTypeDogma,
};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("JSON parse error on line {line}: {source}")]
    Json {
        line: usize,
        source: serde_json::Error,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Strip a leading UTF-8 BOM. Defensive: CCP shouldn't emit one, but if it
/// ever does we'd otherwise fail with a confusing JSON parse error.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
}

/// Parse `mapSolarSystems.jsonl` into `System` rows, deriving `sec_class`.
pub fn parse_systems(reader: impl BufRead) -> Result<Vec<System>, ParseError> {
    let mut systems = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let line = strip_bom(line.trim());
        if line.is_empty() {
            continue;
        }
        let raw: RawSystem = serde_json::from_str(line).map_err(|e| ParseError::Json {
            line: i + 1,
            source: e,
        })?;
        systems.push(system_from_raw(raw));
    }
    Ok(systems)
}

/// Parse `mapStargates.jsonl` into deduplicated undirected `(a, b)` pairs with
/// `a <= b`. The SDE stores both directed halves of each link; we collapse
/// them in a single streaming pass.
pub fn parse_gate_pairs(reader: impl BufRead) -> Result<Vec<(i64, i64)>, ParseError> {
    let mut seen: FxHashSet<(i64, i64)> = FxHashSet::default();
    let mut pairs = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let line = strip_bom(line.trim());
        if line.is_empty() {
            continue;
        }
        let raw: RawStargate = serde_json::from_str(line).map_err(|e| ParseError::Json {
            line: i + 1,
            source: e,
        })?;
        let a = raw.solar_system_id;
        let b = raw.destination.solar_system_id;
        let key = if a <= b { (a, b) } else { (b, a) };
        if seen.insert(key) {
            pairs.push(key);
        }
    }
    Ok(pairs)
}

/// Build the hull catalog by streaming `types.jsonl` then `typeDogma.jsonl`.
///
/// `types.jsonl` is ~150 MB (every item in EVE); we **never materialise it**.
/// Pass 1 streams it line by line, retaining only the lean metadata
/// (`name`, `groupID`) of **published** types, keyed by typeID. Pass 2 streams
/// `typeDogma.jsonl`, and for each row reads attribute 867 (range) when the type
/// was retained, plus attribute 870 from the JDC skill row. A hull enters the
/// catalog only if it has both a retained published `types` row **and** a 867
/// value — i.e. it is genuinely jump-capable.
pub fn parse_hull_catalog(
    types_reader: impl BufRead,
    dogma_reader: impl BufRead,
) -> Result<RawHullCatalog, ParseError> {
    // Pass 1: published types only → (name, group_id) by typeID. Holds ~tens of
    // thousands of lean entries at most, not the 150 MB source.
    let mut published: FxHashMap<i64, (String, i64)> = FxHashMap::default();
    for (i, line) in types_reader.lines().enumerate() {
        let line = line?;
        let line = strip_bom(line.trim());
        if line.is_empty() {
            continue;
        }
        let raw: RawType = serde_json::from_str(line).map_err(|e| ParseError::Json {
            line: i + 1,
            source: e,
        })?;
        if raw.published {
            published.insert(raw.id, (raw.name.en, raw.group_id));
        }
    }

    // Pass 2: read 867 for retained types and 870 from the JDC skill row.
    let mut hulls = Vec::new();
    let mut jdc_bonus_per_level: Option<f64> = None;
    for (i, line) in dogma_reader.lines().enumerate() {
        let line = line?;
        let line = strip_bom(line.trim());
        if line.is_empty() {
            continue;
        }
        let raw: RawTypeDogma = serde_json::from_str(line).map_err(|e| ParseError::Json {
            line: i + 1,
            source: e,
        })?;

        if raw.id == JDC_SKILL_TYPE_ID
            && let Some(attr) = raw
                .dogma_attributes
                .iter()
                .find(|a| a.attribute_id == ATTR_JUMP_DRIVE_RANGE_BONUS)
        {
            // SDE stores the bonus as a percent (20.0); store as a fraction.
            jdc_bonus_per_level = Some(attr.value / 100.0);
        }

        if let Some((name, group_id)) = published.get(&raw.id)
            && let Some(attr) = raw
                .dogma_attributes
                .iter()
                .find(|a| a.attribute_id == ATTR_JUMP_DRIVE_RANGE)
        {
            hulls.push(RawHull {
                type_id: raw.id,
                name: name.clone(),
                group_id: *group_id,
                base_ly: attr.value,
            });
        }
    }

    Ok(RawHullCatalog {
        hulls,
        jdc_bonus_per_level,
    })
}

fn system_from_raw(raw: RawSystem) -> System {
    let is_wormhole = raw.region_id >= 11_000_000;
    let sec_class = classify_security(raw.security_status, is_wormhole);
    System {
        id: raw.id,
        name: raw.name.en,
        security: raw.security_status,
        sec_class,
        coords: [raw.position.x, raw.position.y, raw.position.z],
        region_id: raw.region_id,
        constellation_id: raw.constellation_id,
    }
}

/// Classify a system's security. Wormhole systems are always `Wormhole`
/// regardless of raw security. Highsec is `securityStatus >= 0.45` — EVE
/// displays security rounded to one decimal, so raw 0.45 shows as 0.5.
fn classify_security(sec: f32, is_wormhole: bool) -> SecClass {
    if is_wormhole {
        return SecClass::Wormhole;
    }
    if sec >= 0.45 {
        SecClass::Highsec
    } else if sec > 0.0 {
        SecClass::Lowsec
    } else {
        SecClass::Nullsec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_security_highsec() {
        assert!(matches!(classify_security(0.945, false), SecClass::Highsec));
        // Boundary: raw 0.45 is the lowest highsec value.
        assert!(matches!(classify_security(0.45, false), SecClass::Highsec));
    }

    #[test]
    fn classify_security_lowsec() {
        assert!(matches!(classify_security(0.4, false), SecClass::Lowsec));
        assert!(matches!(classify_security(0.1, false), SecClass::Lowsec));
    }

    #[test]
    fn classify_security_nullsec() {
        assert!(matches!(classify_security(0.0, false), SecClass::Nullsec));
        assert!(matches!(classify_security(-0.5, false), SecClass::Nullsec));
    }

    #[test]
    fn classify_security_wormhole_overrides_raw() {
        assert!(matches!(classify_security(0.0, true), SecClass::Wormhole));
        assert!(matches!(classify_security(0.9, true), SecClass::Wormhole));
    }

    #[test]
    fn parse_systems_from_fixture() {
        let jsonl = r#"{"_key": 30000142, "constellationID": 20000020, "name": {"en": "Jita", "de": "Jita"}, "position": {"x": -1.29e+17, "y": 6.07e+16, "z": 1.17e+17}, "regionID": 10000002, "securityStatus": 0.945913, "stargateIDs": [50001249]}
{"_key": 30000144, "constellationID": 20000020, "name": {"en": "Perimeter", "de": "Perimeter"}, "position": {"x": -1.43e+17, "y": 6.49e+16, "z": 1.04e+17}, "regionID": 10000002, "securityStatus": 0.953123, "stargateIDs": [50002185]}
"#;
        let systems = parse_systems(jsonl.as_bytes()).unwrap();
        assert_eq!(systems.len(), 2);
        assert_eq!(systems[0].name, "Jita");
        assert_eq!(systems[0].id, 30000142);
        assert!(matches!(systems[0].sec_class, SecClass::Highsec));
        assert!(!systems[0].is_wormhole());
    }

    #[test]
    fn parse_gate_pairs_deduplicates() {
        // Two directed gates forming one bidirectional Jita↔Perimeter link.
        let jsonl = r#"{"_key": 50001249, "solarSystemID": 30000142, "destination": {"solarSystemID": 30000144, "stargateID": 50002185}, "position": {"x": 0, "y": 0, "z": 0}, "typeID": 16}
{"_key": 50002185, "solarSystemID": 30000144, "destination": {"solarSystemID": 30000142, "stargateID": 50001249}, "position": {"x": 0, "y": 0, "z": 0}, "typeID": 16}
"#;
        let pairs = parse_gate_pairs(jsonl.as_bytes()).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0], (30000142, 30000144));
    }

    #[test]
    fn parse_systems_handles_utf8_bom() {
        let jsonl = "\u{FEFF}{\"_key\": 30000142, \"constellationID\": 20000020, \"name\": {\"en\": \"Jita\"}, \"position\": {\"x\": 0.0, \"y\": 0.0, \"z\": 0.0}, \"regionID\": 10000002, \"securityStatus\": 0.95}\n";
        let systems = parse_systems(jsonl.as_bytes()).unwrap();
        assert_eq!(systems.len(), 1);
        assert_eq!(systems[0].name, "Jita");
    }

    #[test]
    fn parse_systems_reports_line_on_bad_json() {
        let jsonl = "{\"_key\": 1, \"name\": {\"en\": \"A\"}, \"position\": {\"x\":0,\"y\":0,\"z\":0}, \"regionID\":10000002, \"constellationID\":1, \"securityStatus\":0.9}\n{ broken\n";
        let err = parse_systems(jsonl.as_bytes()).unwrap_err();
        match err {
            ParseError::Json { line, .. } => assert_eq!(line, 2),
            other => panic!("expected Json error, got {other:?}"),
        }
    }

    #[test]
    fn parse_hull_catalog_joins_and_filters() {
        // A published jump hull (867 present) → kept. A published hull with 868
        // but no 867 → excluded (868 is fuel, not range). An unpublished hull
        // with 867 → excluded. The JDC skill row's 870 → read as the bonus.
        let types = r#"{"_key": 22430, "groupID": 898, "name": {"en": "Sin"}, "published": true}
{"_key": 670, "groupID": 30, "name": {"en": "Capsule"}, "published": true}
{"_key": 99999, "groupID": 898, "name": {"en": "Unpublished Blops"}, "published": false}
{"_key": 21611, "groupID": 257, "name": {"en": "Jump Drive Calibration"}, "published": true}
"#;
        let dogma = r#"{"_key": 22430, "dogmaAttributes": [{"attributeID": 867, "value": 4.0}, {"attributeID": 868, "value": 1000.0}]}
{"_key": 670, "dogmaAttributes": [{"attributeID": 868, "value": 5.0}]}
{"_key": 99999, "dogmaAttributes": [{"attributeID": 867, "value": 4.0}]}
{"_key": 21611, "dogmaAttributes": [{"attributeID": 870, "value": 20.0}]}
"#;
        let cat = parse_hull_catalog(types.as_bytes(), dogma.as_bytes()).unwrap();
        assert_eq!(cat.hulls.len(), 1, "only the published 867-bearing hull");
        let sin = &cat.hulls[0];
        assert_eq!(sin.type_id, 22430);
        assert_eq!(sin.name, "Sin");
        assert_eq!(sin.group_id, 898);
        assert_eq!(sin.base_ly, 4.0, "base range from 867, not 868");
        // JDC 870 (20.0 percent) is stored as the 0.20 per-level fraction.
        assert_eq!(cat.jdc_bonus_per_level, Some(0.20));
    }

    #[test]
    fn parse_hull_catalog_handles_missing_jdc_row() {
        let types = r#"{"_key": 22430, "groupID": 898, "name": {"en": "Sin"}, "published": true}
"#;
        let dogma = r#"{"_key": 22430, "dogmaAttributes": [{"attributeID": 867, "value": 4.0}]}
"#;
        let cat = parse_hull_catalog(types.as_bytes(), dogma.as_bytes()).unwrap();
        assert_eq!(cat.hulls.len(), 1);
        assert!(cat.jdc_bonus_per_level.is_none());
    }

    #[test]
    fn wormhole_system_detected_by_region_id() {
        // J-space region IDs start at 11_000_000.
        let jsonl = r#"{"_key": 31000001, "constellationID": 21000001, "name": {"en": "J100001", "de": "J100001"}, "position": {"x": 0.0, "y": 0.0, "z": 0.0}, "regionID": 11000001, "securityStatus": -0.99, "stargateIDs": []}
"#;
        let systems = parse_systems(jsonl.as_bytes()).unwrap();
        assert!(systems[0].is_wormhole());
        assert!(matches!(systems[0].sec_class, SecClass::Wormhole));
    }
}
