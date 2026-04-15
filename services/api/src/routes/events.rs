use axum::{
    extract::{Path, Query, State},
    Json,
};
use tracing::debug;

use crate::{
    cache,
    error::RequestError,
    model::{EventListResponse, EventResponse, EventsQuery},
    routes::AppState,
};

// ─── Input validation ─────────────────────────────────────────────────────────

/// Validate all query parameters and return a `RequestError` for the first
/// violation found.
///
/// Extracted from the handler so that the logic can be unit-tested without
/// an Axum runtime or AppState.
pub(crate) fn validate_events_query(query: &EventsQuery) -> Result<(), RequestError> {
    // ── NaN / Infinity guards ─────────────────────────────────────────────────
    // All six float parameters must be finite. NaN comparisons silently return
    // false, which bypasses every range check below and passes a PostgreSQL NaN
    // to the query, where `magnitude >= NaN` returns zero rows with no error.
    for (name, val) in [
        ("min_magnitude", query.min_magnitude),
        ("max_magnitude", query.max_magnitude),
        ("min_lat", query.min_lat),
        ("max_lat", query.max_lat),
        ("min_lon", query.min_lon),
        ("max_lon", query.max_lon),
    ] {
        if let Some(v) = val {
            if !v.is_finite() {
                return Err(RequestError::BadParam {
                    param: name,
                    detail: format!("{} is not a finite number", v),
                });
            }
        }
    }

    // ── Magnitude range ───────────────────────────────────────────────────────
    if let (Some(min), Some(max)) = (query.min_magnitude, query.max_magnitude) {
        if min > max {
            return Err(RequestError::BadParam {
                param: "min_magnitude",
                detail: format!("{} exceeds max_magnitude {}", min, max),
            });
        }
    }

    // ── Time range ────────────────────────────────────────────────────────────
    if let (Some(start), Some(end)) = (query.start_time, query.end_time) {
        if start > end {
            return Err(RequestError::BadParam {
                param: "start_time",
                detail: format!(
                    "{} is after end_time {}",
                    start.to_rfc3339(),
                    end.to_rfc3339()
                ),
            });
        }
    }

    // ── Bounding box ─────────────────────────────────────────────────────────
    // Either all four corners must be present or none. A partial bbox returns
    // 400 rather than silently ignoring the supplied parameters.
    let bbox_provided = [query.min_lat, query.max_lat, query.min_lon, query.max_lon]
        .iter()
        .any(|v| v.is_some());

    if let (Some(min_lat), Some(max_lat), Some(min_lon), Some(max_lon)) =
        (query.min_lat, query.max_lat, query.min_lon, query.max_lon)
    {
        // All four corners are present — validate ranges and ordering.

        // ── Individual coordinate ranges ──────────────────────────────────────
        if !(-90.0..=90.0).contains(&min_lat) {
            return Err(RequestError::BadParam {
                param: "min_lat",
                detail: format!("{} is not in [-90, 90]", min_lat),
            });
        }
        if !(-90.0..=90.0).contains(&max_lat) {
            return Err(RequestError::BadParam {
                param: "max_lat",
                detail: format!("{} is not in [-90, 90]", max_lat),
            });
        }
        if !(-180.0..=180.0).contains(&min_lon) {
            return Err(RequestError::BadParam {
                param: "min_lon",
                detail: format!("{} is not in [-180, 180]", min_lon),
            });
        }
        if !(-180.0..=180.0).contains(&max_lon) {
            return Err(RequestError::BadParam {
                param: "max_lon",
                detail: format!("{} is not in [-180, 180]", max_lon),
            });
        }

        // ── Ordering checks ───────────────────────────────────────────────────
        // PostGIS ST_MakeEnvelope(min_lon, min_lat, max_lon, max_lat) requires
        // min < max for each axis. A reversed bbox produces an empty result
        // with 200 OK, indistinguishable from a legitimately empty region.
        // Anti-meridian crossing (min_lon > max_lon) is not supported by the
        // `&&` operator in Phase 1; split the request at 180° instead.
        if min_lat >= max_lat {
            return Err(RequestError::BadParam {
                param: "min_lat",
                detail: format!("{} must be strictly less than max_lat {}", min_lat, max_lat),
            });
        }
        if min_lon >= max_lon {
            return Err(RequestError::BadParam {
                param: "min_lon",
                detail: format!(
                    "{} must be strictly less than max_lon {} \
                     (anti-meridian crossing is not supported in Phase 1)",
                    min_lon, max_lon
                ),
            });
        }
    } else if bbox_provided {
        // At least one corner was given but not all four.
        return Err(RequestError::BadParam {
            param: "bbox",
            detail: "all four bounding box parameters (min_lat, max_lat, min_lon, max_lon) \
                     must be provided together"
                .to_owned(),
        });
    }

    Ok(())
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

/// List earthquake events with optional filtering and pagination.
///
/// All filter parameters are optional. When a bounding-box filter is used,
/// all four corners (`min_lat`, `max_lat`, `min_lon`, `max_lon`) must be
/// supplied together; a partial bounding box returns `400`. Anti-meridian
/// bounding boxes are not supported in Phase 1.
#[utoipa::path(
    get,
    path = "/v1/events",
    tag = "events",
    params(EventsQuery),
    responses(
        (status = 200, description = "Paginated event list", body = EventListResponse),
        (status = 400, description = "Invalid query parameter"),
    )
)]
pub async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventListResponse>, RequestError> {
    validate_events_query(&query)?;

    // Compute pagination once; the same values go into the DB call and the
    // response body so they can never drift.
    let page = query.page.unwrap_or(EventsQuery::DEFAULT_PAGE).max(1);
    let page_size = query
        .page_size
        .unwrap_or(EventsQuery::DEFAULT_PAGE_SIZE)
        .clamp(1, EventsQuery::MAX_PAGE_SIZE);

    debug!(page, page_size, "Listing events");

    let (events, total) = crate::db::list_events(&state.pool, &query, page, page_size).await?;

    Ok(Json(EventListResponse {
        events,
        page,
        page_size,
        total,
    }))
}

/// Fetch a single earthquake event by its source identifier.
///
/// The lookup checks the Redis cache (key `api:event:{id}`, TTL 300 s)
/// before falling back to PostgreSQL.
#[utoipa::path(
    get,
    path = "/v1/events/{id}",
    tag = "events",
    params(
        ("id" = String, Path, description = "External source identifier (e.g. us6000abcd)")
    ),
    responses(
        (status = 200, description = "Event found",     body = EventResponse),
        (status = 404, description = "Event not found"),
    )
)]
pub async fn get_event(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
) -> Result<Json<EventResponse>, RequestError> {
    let cache_key = cache::event_key(&source_id);

    // Cache hit.
    if let Some(cached) = state.cache.get::<EventResponse>(&cache_key).await {
        state
            .metrics
            .cache_hits_total
            .with_label_values(&["get_event"])
            .inc();
        return Ok(Json(cached));
    }

    state
        .metrics
        .cache_misses_total
        .with_label_values(&["get_event"])
        .inc();

    // Cache miss — query PostgreSQL.
    let event = crate::db::get_event_by_source_id(&state.pool, &source_id)
        .await?
        .ok_or(RequestError::NotFound)?;

    // Populate cache asynchronously; don't block the response on cache write.
    // The spawned task's JoinHandle is intentionally dropped — `Cache::set`
    // already logs on failure, and a cache-write panic should not affect the
    // response already sent to the client.
    let cache = state.cache.clone();
    let ttl = state.config.event_cache_ttl_secs;
    let to_cache = event.clone();
    drop(tokio::spawn(async move {
        cache.set(&cache_key, &to_cache, ttl).await;
    })); // intentional drop: cache failure is non-fatal, logged by Cache::set

    Ok(Json(event))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn q() -> EventsQuery {
        EventsQuery {
            page: None,
            page_size: None,
            start_time: None,
            end_time: None,
            min_magnitude: None,
            max_magnitude: None,
            min_lat: None,
            max_lat: None,
            min_lon: None,
            max_lon: None,
        }
    }

    // ── NaN / Infinity ────────────────────────────────────────────────────────

    #[test]
    fn nan_magnitude_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_magnitude: Some(f64::NAN),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_magnitude",
                ..
            }
        ));
    }

    #[test]
    fn inf_magnitude_rejected() {
        let err = validate_events_query(&EventsQuery {
            max_magnitude: Some(f64::INFINITY),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "max_magnitude",
                ..
            }
        ));
    }

    #[test]
    fn nan_lat_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_lat: Some(f64::NAN),
            max_lat: Some(50.0),
            min_lon: Some(-10.0),
            max_lon: Some(45.0),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_lat",
                ..
            }
        ));
    }

    // ── Magnitude ordering ────────────────────────────────────────────────────

    #[test]
    fn reversed_magnitude_range_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_magnitude: Some(7.0),
            max_magnitude: Some(3.0),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_magnitude",
                ..
            }
        ));
    }

    #[test]
    fn equal_magnitude_bounds_accepted() {
        validate_events_query(&EventsQuery {
            min_magnitude: Some(5.0),
            max_magnitude: Some(5.0),
            ..q()
        })
        .expect("equal bounds should be valid");
    }

    // ── Time ordering ─────────────────────────────────────────────────────────

    #[test]
    fn reversed_time_range_rejected() {
        let now = Utc::now();
        let err = validate_events_query(&EventsQuery {
            start_time: Some(now),
            end_time: Some(now - Duration::hours(1)),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "start_time",
                ..
            }
        ));
    }

    #[test]
    fn equal_time_bounds_accepted() {
        let now = Utc::now();
        validate_events_query(&EventsQuery {
            start_time: Some(now),
            end_time: Some(now),
            ..q()
        })
        .expect("equal time bounds should be valid");
    }

    // ── Bounding box completeness ─────────────────────────────────────────────

    #[test]
    fn partial_bbox_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_lat: Some(35.0),
            // max_lat, min_lon, max_lon absent
            ..q()
        })
        .unwrap_err();
        assert!(matches!(err, RequestError::BadParam { param: "bbox", .. }));
    }

    #[test]
    fn complete_bbox_accepted() {
        validate_events_query(&EventsQuery {
            min_lat: Some(35.0),
            max_lat: Some(72.0),
            min_lon: Some(-10.0),
            max_lon: Some(45.0),
            ..q()
        })
        .expect("valid bbox should pass");
    }

    // ── Bounding box ordering ─────────────────────────────────────────────────

    #[test]
    fn reversed_lat_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_lat: Some(72.0), // north > south
            max_lat: Some(35.0),
            min_lon: Some(-10.0),
            max_lon: Some(45.0),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_lat",
                ..
            }
        ));
    }

    #[test]
    fn equal_lat_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_lat: Some(35.0),
            max_lat: Some(35.0),
            min_lon: Some(-10.0),
            max_lon: Some(45.0),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_lat",
                ..
            }
        ));
    }

    #[test]
    fn anti_meridian_crossing_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_lat: Some(35.0),
            max_lat: Some(72.0),
            min_lon: Some(150.0), // would cross anti-meridian
            max_lon: Some(-150.0),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_lon",
                ..
            }
        ));
    }

    // ── Out-of-range coordinates ──────────────────────────────────────────────

    #[test]
    fn out_of_range_lat_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_lat: Some(-91.0),
            max_lat: Some(72.0),
            min_lon: Some(-10.0),
            max_lon: Some(45.0),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_lat",
                ..
            }
        ));
    }

    #[test]
    fn out_of_range_lon_rejected() {
        let err = validate_events_query(&EventsQuery {
            min_lat: Some(35.0),
            max_lat: Some(72.0),
            min_lon: Some(-181.0),
            max_lon: Some(45.0),
            ..q()
        })
        .unwrap_err();
        assert!(matches!(
            err,
            RequestError::BadParam {
                param: "min_lon",
                ..
            }
        ));
    }

    // ── Empty query passes ────────────────────────────────────────────────────

    #[test]
    fn empty_query_is_valid() {
        validate_events_query(&q()).expect("empty query must pass validation");
    }

    // ── Property-based tests ──────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Any finite magnitude value in [-2, 10] must pass the magnitude check.
        #[test]
        fn valid_magnitude_range_always_passes(
            min in -2.0f64..=10.0f64,
            max in -2.0f64..=10.0f64,
        ) {
            let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
            let query = EventsQuery { min_magnitude: Some(lo), max_magnitude: Some(hi), ..q() };
            prop_assert!(validate_events_query(&query).is_ok());
        }

        /// Any non-finite magnitude value must be rejected.
        #[test]
        fn non_finite_magnitude_always_rejected(bits in any::<u64>()) {
            let v = f64::from_bits(bits);
            if !v.is_finite() {
                let query = EventsQuery { min_magnitude: Some(v), ..q() };
                prop_assert!(validate_events_query(&query).is_err());
            }
        }

        /// A valid bounding box (min < max for both axes, within range) always passes.
        #[test]
        fn valid_bbox_always_passes(
            lat1 in -90.0f64..90.0f64,
            lat2 in -90.0f64..90.0f64,
            lon1 in -180.0f64..180.0f64,
            lon2 in -180.0f64..180.0f64,
        ) {
            // Ensure min < max for both axes (equal values are rejected).
            let (min_lat, max_lat) = if lat1 < lat2 { (lat1, lat2) } else { (lat2, lat1) };
            let (min_lon, max_lon) = if lon1 < lon2 { (lon1, lon2) } else { (lon2, lon1) };
            if min_lat < max_lat && min_lon < max_lon {
                let query = EventsQuery {
                    min_lat: Some(min_lat),
                    max_lat: Some(max_lat),
                    min_lon: Some(min_lon),
                    max_lon: Some(max_lon),
                    ..q()
                };
                prop_assert!(validate_events_query(&query).is_ok());
            }
        }

        /// Latitudes outside [-90, 90] must always be rejected.
        #[test]
        fn out_of_range_lat_always_rejected(lat in 90.001f64..=1000.0f64) {
            // Use a valid lon range so only the lat triggers the error.
            let query = EventsQuery {
                min_lat: Some(lat),
                max_lat: Some(lat + 1.0),
                min_lon: Some(-10.0),
                max_lon: Some(10.0),
                ..q()
            };
            prop_assert!(validate_events_query(&query).is_err());
        }

        /// Longitudes outside [-180, 180] must always be rejected.
        #[test]
        fn out_of_range_lon_always_rejected(lon in 180.001f64..=1000.0f64) {
            let query = EventsQuery {
                min_lat: Some(-10.0),
                max_lat: Some(10.0),
                min_lon: Some(lon),
                max_lon: Some(lon + 1.0),
                ..q()
            };
            prop_assert!(validate_events_query(&query).is_err());
        }
    }
}
