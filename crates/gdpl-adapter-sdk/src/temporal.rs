use crate::{ClockDomain, ClockExchange};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TemporalQuality {
    DirectInterval,
    PeriodicSnapshotUnknownInterval,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricObservation {
    pub clock_domain: ClockDomain,
    pub acquisition_timestamp_us: Option<u64>,
    pub interval_start_us: Option<u64>,
    pub interval_end_us: Option<u64>,
    pub generated_us: Option<u64>,
    pub resolution_us: Option<u64>,
    pub quality: TemporalQuality,
    pub provenance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockModel {
    pub host_origin_us: f64,
    pub offset_us: f64,
    pub rate_difference_ppm: f64,
    pub observation_span_us: u64,
    pub residual_dispersion_us: f64,
    pub model_uncertainty_us: f64,
    pub first_host_us: u64,
    pub last_host_us: u64,
    pub exchanges_used: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    Inside,
    Outside,
    OverlappingBoundary,
    TemporallyAmbiguous,
    TimestampUnavailable,
}

/// Intervals are in one clock domain. An estimate is not a mathematical bound.
pub fn classify_interval(
    start: Option<f64>,
    end: Option<f64>,
    uncertainty: f64,
    window_start: f64,
    window_end: f64,
) -> Attribution {
    let (Some(start), Some(end)) = (start, end) else {
        return Attribution::TimestampUnavailable;
    };
    if !start.is_finite()
        || !end.is_finite()
        || !window_start.is_finite()
        || !window_end.is_finite()
        || !uncertainty.is_finite()
        || uncertainty < 0.0
        || start > end
        || window_start >= window_end
    {
        return Attribution::TemporallyAmbiguous;
    }
    if end + uncertainty < window_start || start - uncertainty >= window_end {
        return Attribution::Outside;
    }
    if start - uncertainty >= window_start && end + uncertainty < window_end {
        return Attribution::Inside;
    }
    Attribution::OverlappingBoundary
}

/// Robust median pairwise slopes across widely separated exchanges; systematic asymmetry is unknown.
pub fn fit_clock(exchanges: &[ClockExchange]) -> Option<ClockModel> {
    let mut points: Vec<(u64, f64, u64)> = exchanges
        .iter()
        .filter_map(|e| {
            ClockExchange::new(
                e.host_send_us,
                e.adapter_receive_us,
                e.adapter_send_us,
                e.host_receive_us,
            )
        })
        .map(|e| {
            (
                e.host_send_us + (e.host_receive_us - e.host_send_us) / 2,
                e.offset_us,
                e.round_trip_us,
            )
        })
        .collect();
    points.sort_by_key(|p| p.0);
    let first = points.first()?.0;
    let last = points.last()?.0;
    if points.len() < 3 || last.checked_sub(first)? < 30_000_000 {
        return None;
    }
    if !points
        .iter()
        .any(|p| p.0 >= first + 10_000_000 && p.0 <= last - 10_000_000)
    {
        return None;
    }
    let mut slopes = Vec::new();
    for (index, a) in points.iter().enumerate() {
        for b in &points[index + 1..] {
            let elapsed = b.0 - a.0;
            if elapsed >= 10_000_000 {
                slopes.push((b.1 - a.1) / elapsed as f64);
            }
        }
    }
    if slopes.len() < 3 {
        return None;
    }
    slopes.sort_by(f64::total_cmp);
    let slope = slopes[slopes.len() / 2];
    if !slope.is_finite() || slope.abs() > 0.001 {
        return None;
    }
    let mut offsets: Vec<f64> = points
        .iter()
        .map(|p| p.1 - slope * (p.0 - first) as f64)
        .collect();
    offsets.sort_by(f64::total_cmp);
    let offset = offsets[offsets.len() / 2];
    let mut residuals: Vec<f64> = points
        .iter()
        .map(|p| (p.1 - (offset + slope * (p.0 - first) as f64)).abs())
        .collect();
    residuals.sort_by(f64::total_cmp);
    let dispersion = residuals[residuals.len() / 2];
    if dispersion > 5_000.0 {
        return None;
    }
    let min_rtt = points.iter().map(|p| p.2).min()?;
    Some(ClockModel {
        host_origin_us: first as f64,
        offset_us: offset,
        rate_difference_ppm: slope * 1_000_000.0,
        observation_span_us: last - first,
        residual_dispersion_us: dispersion,
        model_uncertainty_us: dispersion + min_rtt as f64 / 2.0 + 1.0,
        first_host_us: first,
        last_host_us: last,
        exchanges_used: points.len(),
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClockMapping {
    pub original_timestamp_us: u64,
    pub source: ClockDomain,
    pub estimated_host_us: Option<f64>,
    pub estimated_uncertainty_us: Option<f64>,
    pub valid: bool,
    pub synchronization_age_us: u64,
    pub diagnostic: String,
}

pub fn map_engine_timestamp(
    model: Option<&ClockModel>,
    source: ClockDomain,
    timestamp_us: u64,
    now_host_us: u64,
    max_age_us: u64,
    max_extrapolation_us: u64,
    assume_symmetric: bool,
) -> ClockMapping {
    let mut result = ClockMapping {
        original_timestamp_us: timestamp_us,
        source: source.clone(),
        estimated_host_us: None,
        estimated_uncertainty_us: None,
        valid: false,
        synchronization_age_us: 0,
        diagnostic: "insufficient clock observations".into(),
    };
    if source != ClockDomain::EngineMonotonic {
        result.diagnostic = "unsupported source clock domain".into();
        return result;
    }
    let Some(model) = model else {
        return result;
    };
    result.synchronization_age_us = now_host_us.saturating_sub(model.last_host_us);
    if now_host_us < model.last_host_us || result.synchronization_age_us > max_age_us {
        result.diagnostic = "clock model is stale or from the future".into();
        return result;
    }
    if !assume_symmetric {
        result.diagnostic = "unknown path asymmetry; symmetry assumption not accepted".into();
        return result;
    }
    let slope = model.rate_difference_ppm / 1_000_000.0;
    let host =
        (timestamp_us as f64 - model.offset_us + slope * model.host_origin_us) / (1.0 + slope);
    if !host.is_finite()
        || host < model.first_host_us.saturating_sub(max_extrapolation_us) as f64
        || host > model.last_host_us.saturating_add(max_extrapolation_us) as f64
    {
        result.diagnostic = "timestamp outside allowed model range".into();
        return result;
    }
    if host < model.first_host_us as f64 || host > model.last_host_us as f64 {
        result.diagnostic = "extrapolation uncertainty is not established".into();
        return result;
    }
    result.estimated_host_us = Some(host);
    result.estimated_uncertainty_us = Some(model.model_uncertainty_us);
    result.valid = true;
    result.diagnostic = "estimate assumes symmetric latency; systematic asymmetry is unknown and uncertainty is not a bound".into();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exchange(host: u64, offset: i64) -> ClockExchange {
        ClockExchange::new(
            host,
            (host as i64 + offset + 100) as u64,
            (host as i64 + offset + 110) as u64,
            host + 210,
        )
        .unwrap()
    }
    #[test]
    fn fits_known_offset_and_drift_but_not_short_observations() {
        let samples = [
            exchange(1_000_000, 500),
            exchange(21_000_000, 700),
            exchange(41_000_000, 900),
            exchange(61_000_000, 1100),
        ];
        assert!(fit_clock(&samples[..2]).is_none());
        let model = fit_clock(&samples).unwrap();
        assert!((model.rate_difference_ppm - 10.0).abs() < 0.01);
        let result = map_engine_timestamp(
            Some(&model),
            ClockDomain::EngineMonotonic,
            41_000_900,
            61_001_000,
            2_000_000,
            0,
            true,
        );
        assert!(result.valid);
        assert!((result.estimated_host_us.unwrap() - 41_000_000.0).abs() < 2.0);
        assert!(
            !map_engine_timestamp(
                Some(&model),
                ClockDomain::EngineMonotonic,
                41_000_900,
                70_000_000,
                1,
                0,
                true
            )
            .valid
        );
        assert!(
            !map_engine_timestamp(
                Some(&model),
                ClockDomain::EngineMonotonic,
                41_000_900,
                61_001_000,
                2_000_000,
                0,
                false
            )
            .valid
        );
        assert!(
            !map_engine_timestamp(
                Some(&model),
                ClockDomain::EngineMonotonic,
                100_000_000,
                61_001_000,
                2_000_000,
                0,
                true
            )
            .valid
        );
    }
    #[test]
    fn noisy_drift_and_asymmetric_latency_are_not_guarantees() {
        let samples = [
            exchange(1_000_000, 500),
            exchange(21_000_000, 700),
            exchange(41_000_000, 900),
            exchange(61_000_000, 1100),
            exchange(81_000_000, 5000),
        ];
        let model = fit_clock(&samples).unwrap();
        assert!((model.rate_difference_ppm - 10.0).abs() < 0.1);
        let unknown = map_engine_timestamp(
            Some(&model),
            ClockDomain::ExternalCapture,
            41_000_900,
            81_001_000,
            2_000_000,
            0,
            true,
        );
        assert!(!unknown.valid);
        let no_assumption = map_engine_timestamp(
            Some(&model),
            ClockDomain::EngineMonotonic,
            41_000_900,
            81_001_000,
            2_000_000,
            0,
            false,
        );
        assert!(!no_assumption.valid);
        assert!(no_assumption.diagnostic.contains("asymmetry"));
    }
    #[test]
    fn asymmetric_transport_can_bias_offset() {
        // True engine offset is 500 us; forward transit is 20 us and return transit is 180 us.
        let exchange = ClockExchange::new(1000, 1520, 1530, 1210).unwrap();
        assert_eq!(exchange.offset_us, 420.0);
        assert_ne!(exchange.offset_us, 500.0);
    }
    #[test]
    fn classifies_boundaries_and_unknown_times() {
        assert_eq!(
            classify_interval(Some(12.0), Some(18.0), 1.0, 10.0, 20.0),
            Attribution::Inside
        );
        assert_eq!(
            classify_interval(Some(9.0), Some(12.0), 0.0, 10.0, 20.0),
            Attribution::OverlappingBoundary
        );
        assert_eq!(
            classify_interval(Some(20.0), Some(21.0), 0.0, 10.0, 20.0),
            Attribution::Outside
        );
        assert_eq!(
            classify_interval(None, None, 0.0, 10.0, 20.0),
            Attribution::TimestampUnavailable
        );
        assert_eq!(
            classify_interval(Some(20.0), Some(10.0), 0.0, 0.0, 30.0),
            Attribution::TemporallyAmbiguous
        );
    }
}
