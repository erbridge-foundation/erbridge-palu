//! EVE-Scout integration: a background poller that fetches Thera/Turnur
//! wormhole signatures and holds a parsed snapshot behind an `ArcSwap`. The
//! request path never calls EVE-Scout — handlers read the snapshot.

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// EVE-Scout's public signatures endpoint.
pub const EVE_SCOUT_URL: &str = "https://api.eve-scout.com/v2/public/signatures";

/// Thera and Turnur are the two K-space-connected wormhole hubs EVE-Scout
/// tracks. We partition signatures by their `out_system_id`.
pub const THERA_SYSTEM_ID: i64 = 31000005;
pub const TURNUR_SYSTEM_ID: i64 = 30002086;

/// A raw signature row from the EVE-Scout v2 response. Only the fields we
/// route on are deserialized; the rest are ignored by serde.
#[derive(Debug, Clone, Deserialize)]
pub struct RawSignature {
    pub out_system_id: i64,
    pub in_system_id: i64,
    pub in_system_name: String,
    /// `medium`/`large`/`xlarge`/`capital` — reserved for future ship-fit
    /// filtering; parsed and stored but not enforced.
    #[serde(default)]
    pub max_ship_size: Option<String>,
    pub signature_type: String,
    pub expires_at: DateTime<Utc>,
}

/// A wormhole signature usable as a routing overlay edge.
#[derive(Debug, Clone)]
pub struct Signature {
    /// The hub side (Thera or Turnur).
    pub out_system_id: i64,
    /// The K-space (or WH) endpoint the hub connects to.
    pub in_system_id: i64,
    pub in_system_name: String,
    pub max_ship_size: Option<String>,
    pub expires_at: DateTime<Utc>,
}

impl Signature {
    /// True iff this signature is still live at `now`.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.expires_at > now
    }
}

/// An immutable snapshot of EVE-Scout signatures, partitioned by hub. Served
/// read-only behind an `ArcSwap`; expired sigs are filtered when building each
/// request's overlay (not pruned here, so the counts reflect what was fetched).
#[derive(Debug, Clone, Default)]
pub struct EveScoutSnapshot {
    pub thera: Vec<Signature>,
    pub turnur: Vec<Signature>,
    /// When this snapshot was fetched. `None` for the empty initial snapshot.
    pub fetched_at: Option<DateTime<Utc>>,
}

impl EveScoutSnapshot {
    /// Total signature count across both hubs (for `/health`).
    pub fn sig_count(&self) -> usize {
        self.thera.len() + self.turnur.len()
    }
}

/// Partition raw signatures into a snapshot: keep only `wormhole` entries and
/// split by `out_system_id` (Thera / Turnur). Other origins are dropped.
pub fn partition(raw: Vec<RawSignature>, fetched_at: DateTime<Utc>) -> EveScoutSnapshot {
    let mut thera = Vec::new();
    let mut turnur = Vec::new();
    for r in raw {
        if r.signature_type != "wormhole" {
            continue;
        }
        let sig = Signature {
            out_system_id: r.out_system_id,
            in_system_id: r.in_system_id,
            in_system_name: r.in_system_name,
            max_ship_size: r.max_ship_size,
            expires_at: r.expires_at,
        };
        match sig.out_system_id {
            THERA_SYSTEM_ID => thera.push(sig),
            TURNUR_SYSTEM_ID => turnur.push(sig),
            _ => {}
        }
    }
    EveScoutSnapshot {
        thera,
        turnur,
        fetched_at: Some(fetched_at),
    }
}

/// Fetch the EVE-Scout signatures and parse them into a snapshot. Errors
/// propagate so the poller can log and keep the previous snapshot.
pub async fn fetch_snapshot(client: &reqwest::Client) -> anyhow::Result<EveScoutSnapshot> {
    use anyhow::Context;
    let raw: Vec<RawSignature> = client
        .get(EVE_SCOUT_URL)
        .send()
        .await
        .context("fetching EVE-Scout signatures")?
        .error_for_status()
        .context("EVE-Scout returned error status")?
        .json()
        .await
        .context("parsing EVE-Scout signatures")?;
    Ok(partition(raw, Utc::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(out: i64, in_id: i64, sig_type: &str, expires: DateTime<Utc>) -> RawSignature {
        RawSignature {
            out_system_id: out,
            in_system_id: in_id,
            in_system_name: format!("S{in_id}"),
            max_ship_size: Some("xlarge".into()),
            signature_type: sig_type.into(),
            expires_at: expires,
        }
    }

    #[test]
    fn partition_splits_by_hub() {
        let now = Utc::now();
        let snap = partition(
            vec![
                raw(THERA_SYSTEM_ID, 30004556, "wormhole", now),
                raw(TURNUR_SYSTEM_ID, 30002000, "wormhole", now),
                raw(THERA_SYSTEM_ID, 30004557, "wormhole", now),
            ],
            now,
        );
        assert_eq!(snap.thera.len(), 2);
        assert_eq!(snap.turnur.len(), 1);
        assert_eq!(snap.sig_count(), 3);
        assert!(snap.fetched_at.is_some());
    }

    #[test]
    fn partition_drops_non_wormhole() {
        let now = Utc::now();
        let snap = partition(
            vec![
                raw(THERA_SYSTEM_ID, 1, "wormhole", now),
                raw(THERA_SYSTEM_ID, 2, "data", now),
                raw(THERA_SYSTEM_ID, 3, "relic", now),
            ],
            now,
        );
        assert_eq!(snap.thera.len(), 1);
    }

    #[test]
    fn partition_drops_unknown_origin() {
        let now = Utc::now();
        let snap = partition(vec![raw(30000142, 1, "wormhole", now)], now);
        assert_eq!(snap.sig_count(), 0);
    }

    #[test]
    fn signature_liveness() {
        let now = Utc::now();
        let live = Signature {
            out_system_id: THERA_SYSTEM_ID,
            in_system_id: 1,
            in_system_name: "X".into(),
            max_ship_size: None,
            expires_at: now + chrono::Duration::hours(1),
        };
        let expired = Signature {
            expires_at: now - chrono::Duration::hours(1),
            ..live.clone()
        };
        assert!(live.is_live(now));
        assert!(!expired.is_live(now));
    }

    #[test]
    fn empty_snapshot_has_no_fetch_time() {
        let snap = EveScoutSnapshot::default();
        assert_eq!(snap.sig_count(), 0);
        assert!(snap.fetched_at.is_none());
    }

    #[test]
    fn deserializes_real_eve_scout_fixture() {
        // Offline fixture captured from the live API (28 sigs, 18 Thera / 10
        // Turnur). Ensures the row shape matches reality without a network call.
        let json = include_str!("../tests/fixtures/eve-scout-sigs.json");
        let raw: Vec<RawSignature> = serde_json::from_str(json).unwrap();
        assert_eq!(raw.len(), 28);
        let snap = partition(raw, Utc::now());
        assert_eq!(snap.thera.len(), 18);
        assert_eq!(snap.turnur.len(), 10);
    }
}
