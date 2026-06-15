//! `POST /api/v1/route/system` and `POST /api/v1/route/blops` handlers.

use axum::{Json, extract::State};

use crate::app_state::AppState;
use crate::dto::{
    BlopsRouteRequest, BlopsRouteResponse, GateRouteRequest, GateRouteResponse, RangeHull,
    RangeRequest, RangeResponse, ShipRef,
};
use crate::error::AppError;
use crate::model::{GraphData, SecClass, System};
use crate::range::effective_ly;
use crate::services::blops::{StagingQuery, compute_staging};
use crate::services::range::compute_reachable;
use crate::services::route::{OverlayInputs, compute_gate_route, resolve_system};

/// EVE's Black Ops hull group (`groupID` 898). The worst-hull default is the
/// catalog minimum base range over this group, not a hardcoded value.
pub const BLOPS_GROUP_ID: i64 = 898;

/// Compute a system-to-system route over the SDE gate graph plus the
/// per-request wormhole overlay.
#[utoipa::path(
    post,
    path = "/api/v1/route/system",
    request_body = GateRouteRequest,
    responses(
        (status = 200, description = "Route found", body = GateRouteResponse),
        (status = 400, description = "Unknown system in the request"),
        (status = 404, description = "No route exists under the given overlay"),
    ),
    tag = "routing",
)]
pub async fn route_system(
    State(state): State<AppState>,
    Json(req): Json<GateRouteRequest>,
) -> Result<Json<GateRouteResponse>, AppError> {
    // Load both snapshots once so the whole computation sees a consistent view.
    let graph = state.graph.load();
    let scout = state.eve_scout.load();
    let resp = compute_gate_route(&graph, &scout, &req)?;
    Ok(Json(resp))
}

/// Solve the black-ops staging problem: given a fleet at `from` (A) and a fixed
/// cyno target `to` (B), return the gate route to the fewest-jump in-range
/// staging system ★ plus the bridge leg ★→B and inline fallbacks.
#[utoipa::path(
    post,
    path = "/api/v1/route/blops",
    request_body = BlopsRouteRequest,
    responses(
        (status = 200, description = "Staging route found", body = BlopsRouteResponse),
        (status = 400, description = "Unknown system/hull, invalid jdc_level, or highsec target"),
        (status = 404, description = "No in-range or gate-reachable staging system"),
    ),
    tag = "routing",
)]
pub async fn route_blops(
    State(state): State<AppState>,
    Json(req): Json<BlopsRouteRequest>,
) -> Result<Json<BlopsRouteResponse>, AppError> {
    let graph = state.graph.load();
    let scout = state.eve_scout.load();

    // Blops defaulting + validation are the handler's job (the service is
    // mechanic-agnostic). Resolve the effective range and whether we defaulted.
    let (effective, defaulted) = resolve_effective_ly(&graph, req.ship.as_ref(), req.jdc_level)?;

    let query = StagingQuery {
        from: &req.from,
        to: &req.to,
        effective_ly: effective,
        overlay: OverlayInputs {
            avoid: &req.avoid,
            include_zarzakh: req.include_zarzakh,
            use_wormholes: req.use_wormholes,
            connections: &req.connections,
            include_thera: req.include_thera,
            include_turnur: req.include_turnur,
        },
        preference: req.preference.into(),
    };

    // The directional security rule, as two separate predicates (never merged):
    // B (cyno destination) must not be highsec; ★ (staging origin) may be any
    // K-space class — bridging out of highsec is legal.
    let mut resp = compute_staging(
        &graph,
        &scout,
        &query,
        |b: &System| b.sec_class != SecClass::Highsec,
        |_star: &System| true,
        |b: &System| AppError::CynoTargetHighsec(b.name.clone()),
    )?;

    // Echo the bridge parameters the handler owns.
    resp.jdc_level = req.jdc_level;
    resp.effective_ly = effective;
    resp.defaulted = defaulted;
    Ok(Json(resp))
}

/// Compute the jump-range reachability fan-out: given `from`, a required `ship`,
/// and a required `jdc_level`, return every K-space, non-highsec system within a
/// single jump. Planning-oriented — no overlay, and an empty reachable set is a
/// valid 200 answer rather than an error.
#[utoipa::path(
    post,
    path = "/api/v1/route/range",
    request_body = RangeRequest,
    responses(
        (status = 200, description = "Reachable systems (possibly empty)", body = RangeResponse),
        (status = 400, description = "Unknown system/hull or jdc_level outside 1..=5"),
    ),
    tag = "routing",
)]
pub async fn route_range(
    State(state): State<AppState>,
    Json(req): Json<RangeRequest>,
) -> Result<Json<RangeResponse>, AppError> {
    let graph = state.graph.load();

    validate_jdc_level(req.jdc_level)?;
    let from = resolve_system(&graph, &req.from)?;
    let entry = resolve_hull(&graph, &req.ship)?;

    let effective = effective_ly(entry.base_ly, graph.hulls.bonus_per_level(), req.jdc_level);
    let hull = RangeHull {
        name: entry.name,
        type_id: entry.type_id,
        base_ly: entry.base_ly,
    };

    let resp = compute_reachable(&graph, from, hull, req.jdc_level, effective);
    Ok(Json(resp))
}

/// Validate a required `jdc_level` against the universal rule: every
/// jump-capable hull requires Jump Drive Calibration level 1 at a minimum, so a
/// valid level is `1..=5`. `0` and values above 5 are rejected rather than
/// clamped.
fn validate_jdc_level(jdc_level: u8) -> Result<(), AppError> {
    if !(1..=5).contains(&jdc_level) {
        return Err(AppError::InvalidParam(format!(
            "jdc_level must be 1..=5, got {jdc_level}"
        )));
    }
    Ok(())
}

/// Resolve the effective bridge range (LY) and whether the worst-hull default
/// was applied. Validates `jdc_level` (1..=5) and resolves `ship` against the
/// catalog; an omitted ship uses the catalog minimum over the Black Ops group.
fn resolve_effective_ly(
    graph: &GraphData,
    ship: Option<&ShipRef>,
    jdc_level: u8,
) -> Result<(f64, bool), AppError> {
    validate_jdc_level(jdc_level)?;
    let bonus = graph.hulls.bonus_per_level();
    match ship {
        Some(r) => {
            let entry = resolve_hull(graph, r)?;
            Ok((effective_ly(entry.base_ly, bonus, jdc_level), false))
        }
        None => {
            // Worst Black Ops hull: catalog minimum over the group.
            let base = graph
                .hulls
                .min_base_ly_for_group(BLOPS_GROUP_ID)
                .ok_or_else(|| AppError::UnknownHull("no Black Ops hull in catalog".to_string()))?;
            Ok((effective_ly(base, bonus, jdc_level), true))
        }
    }
}

/// Resolve a `ShipRef` against the catalog, case-insensitively for names.
fn resolve_hull(graph: &GraphData, r: &ShipRef) -> Result<crate::model::HullEntry, AppError> {
    match r {
        ShipRef::Id(id) => graph
            .hulls
            .by_type_id(*id)
            .ok_or_else(|| AppError::UnknownHull(id.to_string())),
        ShipRef::Name(name) => graph
            .hulls
            .by_name(name)
            .ok_or_else(|| AppError::UnknownHull(name.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_graph_data;
    use crate::model::{RawHull, RawHullCatalog, RawSdeData};

    fn graph_with_blops() -> GraphData {
        build_graph_data(
            RawSdeData {
                systems: vec![],
                gate_pairs: vec![],
                hulls: RawHullCatalog {
                    hulls: vec![
                        RawHull {
                            type_id: 22430,
                            name: "Sin".into(),
                            group_id: BLOPS_GROUP_ID,
                            base_ly: 4.0,
                        },
                        RawHull {
                            type_id: 44996,
                            name: "Marshal".into(),
                            group_id: BLOPS_GROUP_ID,
                            base_ly: 3.5,
                        },
                    ],
                    jdc_bonus_per_level: Some(0.20),
                },
            },
            0,
        )
    }

    #[test]
    fn named_hull_resolves_to_its_range() {
        let gd = graph_with_blops();
        let (eff, defaulted) =
            resolve_effective_ly(&gd, Some(&ShipRef::Name("Sin".into())), 5).unwrap();
        // 4.0 base × (1 + 0.20×5) = 8.0.
        assert_eq!(eff, 8.0);
        assert!(!defaulted, "an explicit ship is not a default");
    }

    #[test]
    fn hull_by_type_id_resolves() {
        let gd = graph_with_blops();
        let (eff, _) = resolve_effective_ly(&gd, Some(&ShipRef::Id(44996)), 5).unwrap();
        // Marshal 3.5 × 2.0 = 7.0.
        assert_eq!(eff, 7.0);
    }

    #[test]
    fn omitted_ship_uses_worst_blops_and_flags_default() {
        let gd = graph_with_blops();
        let (eff, defaulted) = resolve_effective_ly(&gd, None, 5).unwrap();
        // Worst (min) base in group 898 is Marshal's 3.5 → 7.0 at JDC 5.
        assert_eq!(eff, 7.0);
        assert!(defaulted, "omitted ship is the worst-hull default");
    }

    #[test]
    fn jdc_level_scales_range() {
        let gd = graph_with_blops();
        // Sin 4.0 at JDC 1 → 4.0 × 1.2 = 4.8; at JDC 3 → 4.0 × 1.6 = 6.4. (JDC 0
        // is no longer a valid input — every jump hull requires JDC 1.)
        let (eff1, _) = resolve_effective_ly(&gd, Some(&ShipRef::Name("Sin".into())), 1).unwrap();
        assert!((eff1 - 4.8).abs() < 1e-9);
        let (eff3, _) = resolve_effective_ly(&gd, Some(&ShipRef::Name("Sin".into())), 3).unwrap();
        assert!((eff3 - 6.4).abs() < 1e-9);
    }

    #[test]
    fn jdc_level_above_five_is_rejected() {
        let gd = graph_with_blops();
        let err = resolve_effective_ly(&gd, None, 6).unwrap_err();
        assert!(matches!(err, AppError::InvalidParam(_)));
    }

    #[test]
    fn jdc_level_zero_is_rejected() {
        // Every jump-capable hull requires JDC 1; 0 is no longer accepted by the
        // blops endpoint either (was 0..=5, now 1..=5).
        let gd = graph_with_blops();
        let err = resolve_effective_ly(&gd, None, 0).unwrap_err();
        assert!(matches!(err, AppError::InvalidParam(_)));
    }

    #[test]
    fn validate_jdc_level_accepts_one_through_five_only() {
        assert!(validate_jdc_level(0).is_err());
        for lvl in 1..=5 {
            assert!(
                validate_jdc_level(lvl).is_ok(),
                "level {lvl} should be valid"
            );
        }
        assert!(validate_jdc_level(6).is_err());
    }

    #[test]
    fn unknown_hull_is_rejected() {
        let gd = graph_with_blops();
        let err = resolve_effective_ly(&gd, Some(&ShipRef::Name("Rifter".into())), 5).unwrap_err();
        assert!(matches!(err, AppError::UnknownHull(s) if s == "Rifter"));
    }

    #[test]
    fn range_resolve_hull_echoes_identity() {
        // The range handler builds its hull echo from the resolved entry, which
        // now carries the canonical name + typeID even for an id lookup.
        let gd = graph_with_blops();
        let by_id = resolve_hull(&gd, &ShipRef::Id(22430)).unwrap();
        assert_eq!(by_id.name, "Sin");
        assert_eq!(by_id.type_id, 22430);
        assert_eq!(by_id.base_ly, 4.0);
        // A case-insensitive name lookup echoes the canonical casing.
        let by_name = resolve_hull(&gd, &ShipRef::Name("sIn".into())).unwrap();
        assert_eq!(by_name.name, "Sin");
        assert_eq!(by_name.type_id, 22430);
    }

    #[test]
    fn range_unknown_hull_is_rejected() {
        let gd = graph_with_blops();
        let err = resolve_hull(&gd, &ShipRef::Name("Rifter".into())).unwrap_err();
        assert!(matches!(err, AppError::UnknownHull(s) if s == "Rifter"));
    }

    #[test]
    fn no_blops_in_catalog_defaults_to_unknown_hull() {
        // Empty catalog: the worst-hull default has nothing to fall back to.
        let gd = build_graph_data(
            RawSdeData {
                systems: vec![],
                gate_pairs: vec![],
                hulls: Default::default(),
            },
            0,
        );
        let err = resolve_effective_ly(&gd, None, 5).unwrap_err();
        assert!(matches!(err, AppError::UnknownHull(_)));
    }
}
